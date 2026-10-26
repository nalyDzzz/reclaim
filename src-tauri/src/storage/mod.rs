//! Disk usage analyzer: a parallel directory walk that builds a compact
//! in-memory tree the UI can drill into without re-scanning.

mod categories;

use std::cmp::Reverse;
use std::collections::{BinaryHeap, HashMap, VecDeque};
use std::fs;
use std::os::windows::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::Relaxed};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use rayon::prelude::*;
use serde::Serialize;
use tauri::State;
use tauri::ipc::Channel;

use crate::error::CmdResult;
use crate::paths::{self, is_cloud_placeholder};

pub use categories::category_for;

const NO_PARENT: u32 = u32::MAX;
const LARGEST_FILES_KEPT: usize = 500;

struct Node {
    name: Box<str>,
    size: u64,
    /// Files in this subtree (1 for a file).
    files: u64,
    parent: u32,
    first_child: u32,
    child_count: u32,
    /// Last write time as Windows FILETIME ticks.
    modified: u64,
    is_dir: bool,
    deleted: bool,
    inaccessible: bool,
}

pub struct ScanTree {
    root_path: PathBuf,
    nodes: Vec<Node>,
    largest_files: Vec<u32>,
    types: HashMap<Box<str>, TypeAgg>,
    duration_ms: u64,
    errors: u64,
}

#[derive(Default, Clone, Copy)]
struct TypeAgg {
    bytes: u64,
    count: u64,
}

#[derive(Default)]
pub struct ScanState {
    tree: Mutex<Option<ScanTree>>,
    cancel: Mutex<Option<Arc<AtomicBool>>>,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanProgress {
    files: u64,
    dirs: u64,
    bytes: u64,
    current: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScanSummary {
    root_path: String,
    root_id: u32,
    total_bytes: u64,
    files: u64,
    dirs: u64,
    errors: u64,
    duration_ms: u64,
    cancelled: bool,
}

struct ScanCtx {
    cancel: Arc<AtomicBool>,
    files: AtomicU64,
    dirs: AtomicU64,
    bytes: AtomicU64,
    errors: AtomicU64,
    current: Mutex<String>,
}

struct RawFile {
    name: Box<str>,
    size: u64,
    modified: u64,
}

#[derive(Default)]
struct RawDir {
    name: Box<str>,
    size: u64,
    files: u64,
    modified: u64,
    inaccessible: bool,
    file_entries: Vec<RawFile>,
    dirs: Vec<RawDir>,
}

fn scan_dir(path: &Path, name: Box<str>, modified: u64, ctx: &ScanCtx) -> RawDir {
    let mut dir = RawDir { name, modified, ..Default::default() };
    if ctx.cancel.load(Relaxed) {
        return dir;
    }
    let entries = match fs::read_dir(path) {
        Ok(entries) => entries,
        Err(_) => {
            dir.inaccessible = true;
            ctx.errors.fetch_add(1, Relaxed);
            return dir;
        }
    };
    let mut subdirs = Vec::new();
    for entry in entries.flatten() {
        // On Windows this metadata comes from the directory listing itself,
        // so it costs no extra system call per file.
        let Ok(meta) = entry.metadata() else { continue };
        // Junctions and symlinks would double count (or loop), so skip them.
        if meta.file_type().is_symlink() {
            continue;
        }
        let name: Box<str> = entry.file_name().to_string_lossy().into();
        if meta.is_dir() {
            subdirs.push((entry.path(), name, meta.last_write_time()));
        } else {
            let size = if is_cloud_placeholder(meta.file_attributes()) { 0 } else { meta.len() };
            dir.size += size;
            dir.files += 1;
            dir.file_entries.push(RawFile { name, size, modified: meta.last_write_time() });
        }
    }
    ctx.files.fetch_add(dir.files, Relaxed);
    ctx.bytes.fetch_add(dir.size, Relaxed);
    if ctx.dirs.fetch_add(1, Relaxed).is_multiple_of(32)
        && let Ok(mut current) = ctx.current.try_lock()
    {
        current.clear();
        current.push_str(&path.to_string_lossy());
    }
    dir.dirs = subdirs.into_par_iter().map(|(p, n, m)| scan_dir(&p, n, m, ctx)).collect();
    for d in &dir.dirs {
        dir.size += d.size;
        dir.files += d.files;
    }
    dir
}

fn extension_of(name: &str) -> Box<str> {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 12 => name[i + 1..].to_lowercase().into(),
        _ => "".into(),
    }
}

/// Flattens the raw tree breadth-first so each directory's children sit in
/// one contiguous, size-sorted slice.
fn build_tree(root_path: PathBuf, raw: RawDir) -> ScanTree {
    let mut nodes = Vec::with_capacity(raw.files as usize + 1024);
    nodes.push(Node {
        name: root_path.to_string_lossy().into(),
        size: raw.size,
        files: raw.files,
        parent: NO_PARENT,
        first_child: 0,
        child_count: 0,
        modified: raw.modified,
        is_dir: true,
        deleted: false,
        inaccessible: raw.inaccessible,
    });
    let mut types: HashMap<Box<str>, TypeAgg> = HashMap::new();
    let mut largest: BinaryHeap<Reverse<(u64, u32)>> = BinaryHeap::with_capacity(LARGEST_FILES_KEPT + 1);
    let mut queue = VecDeque::from([(0u32, raw)]);

    enum Child {
        Dir(RawDir),
        File(RawFile),
    }

    while let Some((idx, dir)) = queue.pop_front() {
        let mut children: Vec<Child> = dir.dirs.into_iter().map(Child::Dir).chain(dir.file_entries.into_iter().map(Child::File)).collect();
        children.sort_unstable_by_key(|c| {
            Reverse(match c {
                Child::Dir(d) => d.size,
                Child::File(f) => f.size,
            })
        });
        let first = nodes.len() as u32;
        nodes[idx as usize].first_child = first;
        nodes[idx as usize].child_count = children.len() as u32;
        for child in children {
            let child_idx = nodes.len() as u32;
            match child {
                Child::Dir(mut d) => {
                    nodes.push(Node {
                        name: std::mem::take(&mut d.name),
                        size: d.size,
                        files: d.files,
                        parent: idx,
                        first_child: 0,
                        child_count: 0,
                        modified: d.modified,
                        is_dir: true,
                        deleted: false,
                        inaccessible: d.inaccessible,
                    });
                    queue.push_back((child_idx, d));
                }
                Child::File(f) => {
                    let agg = types.entry(extension_of(&f.name)).or_default();
                    agg.bytes += f.size;
                    agg.count += 1;
                    largest.push(Reverse((f.size, child_idx)));
                    if largest.len() > LARGEST_FILES_KEPT {
                        largest.pop();
                    }
                    nodes.push(Node {
                        name: f.name,
                        size: f.size,
                        files: 1,
                        parent: idx,
                        first_child: 0,
                        child_count: 0,
                        modified: f.modified,
                        is_dir: false,
                        deleted: false,
                        inaccessible: false,
                    });
                }
            }
        }
    }
    let mut largest_files: Vec<(u64, u32)> = largest.into_iter().map(|Reverse(x)| x).collect();
    largest_files.sort_unstable_by_key(|&(size, _)| Reverse(size));
    ScanTree { root_path, nodes, largest_files: largest_files.into_iter().map(|(_, id)| id).collect(), types, duration_ms: 0, errors: 0 }
}

impl ScanTree {
    fn node(&self, id: u32) -> anyhow::Result<&Node> {
        self.nodes.get(id as usize).filter(|n| !n.deleted).context("That item is no longer in the scan")
    }

    fn path_of(&self, id: u32) -> PathBuf {
        let mut parts = Vec::new();
        let mut cur = id;
        while cur != 0 && cur != NO_PARENT {
            let n = &self.nodes[cur as usize];
            parts.push(&*n.name);
            cur = n.parent;
        }
        let mut path = self.root_path.clone();
        for part in parts.iter().rev() {
            path.push(part);
        }
        path
    }

    fn children(&self, id: u32) -> impl Iterator<Item = (u32, &Node)> {
        let n = &self.nodes[id as usize];
        (n.first_child..n.first_child + n.child_count).map(|i| (i, &self.nodes[i as usize])).filter(|(_, n)| !n.deleted)
    }

    fn item(&self, id: u32) -> NodeItem {
        let n = &self.nodes[id as usize];
        NodeItem {
            id,
            name: n.name.to_string(),
            size: n.size,
            files: n.files,
            is_dir: n.is_dir,
            child_count: if n.is_dir { self.children(id).count() as u32 } else { 0 },
            modified: filetime_to_unix_ms(n.modified),
            inaccessible: n.inaccessible,
        }
    }

    fn dir_count(&self) -> u64 {
        self.nodes.iter().filter(|n| n.is_dir && !n.deleted).count() as u64
    }

    /// Marks a node (and its subtree) deleted and subtracts it from every ancestor.
    fn remove(&mut self, id: u32) {
        let (size, files, parent) = {
            let n = &self.nodes[id as usize];
            (n.size, n.files, n.parent)
        };
        let mut stack = vec![id];
        while let Some(cur) = stack.pop() {
            if self.nodes[cur as usize].deleted {
                continue; // removed earlier; already subtracted
            }
            let (first, count, is_dir) = {
                let n = &mut self.nodes[cur as usize];
                n.deleted = true;
                (n.first_child, n.child_count, n.is_dir)
            };
            if is_dir {
                stack.extend(first..first + count);
            } else {
                let n = &self.nodes[cur as usize];
                if let Some(agg) = self.types.get_mut(&extension_of(&n.name)) {
                    agg.bytes = agg.bytes.saturating_sub(n.size);
                    agg.count = agg.count.saturating_sub(1);
                }
            }
        }
        let mut cur = parent;
        while cur != NO_PARENT {
            let n = &mut self.nodes[cur as usize];
            n.size = n.size.saturating_sub(size);
            n.files = n.files.saturating_sub(files);
            cur = n.parent;
        }
    }
}

fn filetime_to_unix_ms(ft: u64) -> i64 {
    if ft == 0 {
        return 0;
    }
    (ft / 10_000) as i64 - 11_644_473_600_000
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeItem {
    id: u32,
    name: String,
    size: u64,
    files: u64,
    is_dir: bool,
    child_count: u32,
    modified: i64,
    inaccessible: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Crumb {
    id: u32,
    name: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeDetails {
    node: NodeItem,
    path: String,
    breadcrumbs: Vec<Crumb>,
    children: Vec<NodeItem>,
    /// Children beyond `limit`, summarized.
    hidden_count: u32,
    hidden_bytes: u64,
}

#[tauri::command]
pub async fn start_scan(path: String, on_progress: Channel<ScanProgress>, state: State<'_, ScanState>) -> CmdResult<ScanSummary> {
    let root = PathBuf::from(&path);
    crate::error::ensure!(root.is_dir(), "{path} isn't a folder that can be scanned");
    let cancel = Arc::new(AtomicBool::new(false));
    if let Some(previous) = state.cancel.lock().unwrap().replace(cancel.clone()) {
        previous.store(true, Relaxed);
    }
    *state.tree.lock().unwrap() = None; // free the previous scan's memory first

    let ctx = Arc::new(ScanCtx {
        cancel: cancel.clone(),
        files: AtomicU64::new(0),
        dirs: AtomicU64::new(0),
        bytes: AtomicU64::new(0),
        errors: AtomicU64::new(0),
        current: Mutex::new(path.clone()),
    });
    let done = Arc::new(AtomicBool::new(false));

    let reporter = {
        let (ctx, done) = (ctx.clone(), done.clone());
        std::thread::spawn(move || {
            while !done.load(Relaxed) {
                let progress = ScanProgress {
                    files: ctx.files.load(Relaxed),
                    dirs: ctx.dirs.load(Relaxed),
                    bytes: ctx.bytes.load(Relaxed),
                    current: ctx.current.lock().map(|c| c.clone()).unwrap_or_default(),
                };
                if on_progress.send(progress).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(120));
            }
        })
    };

    let scan_root = root.clone();
    let scan_ctx = ctx.clone();
    let tree = tauri::async_runtime::spawn_blocking(move || {
        let started = Instant::now();
        let modified = fs::metadata(&scan_root).map(|m| m.last_write_time()).unwrap_or(0);
        let raw = scan_dir(&scan_root, "".into(), modified, &scan_ctx);
        let mut tree = build_tree(scan_root, raw);
        tree.duration_ms = started.elapsed().as_millis() as u64;
        tree.errors = scan_ctx.errors.load(Relaxed);
        tree
    })
    .await;
    done.store(true, Relaxed);
    let _ = reporter.join();
    let tree = tree?;

    let cancelled = cancel.load(Relaxed);
    let root_node = &tree.nodes[0];
    let summary = ScanSummary {
        root_path: tree.root_path.to_string_lossy().to_string(),
        root_id: 0,
        total_bytes: root_node.size,
        files: root_node.files,
        dirs: tree.dir_count(),
        errors: tree.errors,
        duration_ms: tree.duration_ms,
        cancelled,
    };
    if !cancelled {
        *state.tree.lock().unwrap() = Some(tree);
    }
    Ok(summary)
}

#[tauri::command]
pub fn cancel_scan(state: State<'_, ScanState>) {
    if let Some(cancel) = state.cancel.lock().unwrap().as_ref() {
        cancel.store(true, Relaxed);
    }
}

fn with_tree<T>(state: &ScanState, f: impl FnOnce(&ScanTree) -> anyhow::Result<T>) -> CmdResult<T> {
    let guard = state.tree.lock().unwrap();
    let tree = guard.as_ref().context("Scan a drive or folder first")?;
    Ok(f(tree)?)
}

#[tauri::command]
pub fn scan_node(id: u32, limit: Option<u32>, state: State<'_, ScanState>) -> CmdResult<NodeDetails> {
    with_tree(&state, |tree| {
        let node = tree.node(id)?;
        let limit = limit.unwrap_or(500) as usize;
        let mut children = Vec::new();
        let (mut hidden_count, mut hidden_bytes) = (0u32, 0u64);
        if node.is_dir {
            for (i, (child_id, child)) in tree.children(id).enumerate() {
                if i < limit {
                    children.push(tree.item(child_id));
                } else {
                    hidden_count += 1;
                    hidden_bytes += child.size;
                }
            }
        }
        let mut breadcrumbs = Vec::new();
        let mut cur = id;
        while cur != NO_PARENT {
            let n = &tree.nodes[cur as usize];
            breadcrumbs.push(Crumb { id: cur, name: n.name.to_string() });
            cur = n.parent;
        }
        breadcrumbs.reverse();
        Ok(NodeDetails {
            node: tree.item(id),
            path: tree.path_of(id).to_string_lossy().to_string(),
            breadcrumbs,
            children,
            hidden_count,
            hidden_bytes,
        })
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreemapNode {
    id: u32,
    name: String,
    size: u64,
    is_dir: bool,
    /// Combined size of children too small to draw individually.
    other: u64,
    children: Vec<TreemapNode>,
}

fn treemap_node(tree: &ScanTree, id: u32, depth: u8, min_size: u64) -> TreemapNode {
    let n = &tree.nodes[id as usize];
    let mut out = TreemapNode { id, name: n.name.to_string(), size: n.size, is_dir: n.is_dir, other: 0, children: Vec::new() };
    if depth == 0 || !n.is_dir {
        return out;
    }
    for (child_id, child) in tree.children(id) {
        if child.size >= min_size && out.children.len() < 160 {
            out.children.push(treemap_node(tree, child_id, depth - 1, min_size));
        } else {
            out.other += child.size;
        }
    }
    out
}

/// Nested size data for drawing a treemap of `id`, `depth` levels deep.
#[tauri::command]
pub fn scan_treemap(id: u32, depth: Option<u8>, state: State<'_, ScanState>) -> CmdResult<TreemapNode> {
    with_tree(&state, |tree| {
        let node = tree.node(id)?;
        let min_size = (node.size / 1200).max(1);
        Ok(treemap_node(tree, id, depth.unwrap_or(3).min(5), min_size))
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    id: u32,
    name: String,
    path: String,
    size: u64,
    modified: i64,
    category: &'static str,
}

#[tauri::command]
pub fn scan_largest_files(limit: Option<u32>, state: State<'_, ScanState>) -> CmdResult<Vec<FileItem>> {
    with_tree(&state, |tree| {
        Ok(tree
            .largest_files
            .iter()
            .filter(|&&id| !tree.nodes[id as usize].deleted)
            .take(limit.unwrap_or(200) as usize)
            .map(|&id| {
                let n = &tree.nodes[id as usize];
                FileItem {
                    id,
                    name: n.name.to_string(),
                    path: tree.path_of(id).to_string_lossy().to_string(),
                    size: n.size,
                    modified: filetime_to_unix_ms(n.modified),
                    category: category_for(&extension_of(&n.name)),
                }
            })
            .collect())
    })
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TypeStat {
    extension: String,
    category: &'static str,
    bytes: u64,
    count: u64,
}

#[tauri::command]
pub fn scan_file_types(state: State<'_, ScanState>) -> CmdResult<Vec<TypeStat>> {
    with_tree(&state, |tree| {
        let mut stats: Vec<TypeStat> = tree
            .types
            .iter()
            .filter(|(_, agg)| agg.count > 0)
            .map(|(ext, agg)| TypeStat { extension: ext.to_string(), category: category_for(ext), bytes: agg.bytes, count: agg.count })
            .collect();
        stats.sort_unstable_by_key(|s| Reverse(s.bytes));
        Ok(stats)
    })
}

/// Deletes a scanned file or folder. `permanent` skips the Recycle Bin,
/// which is the only way to actually free space on the same drive.
#[tauri::command]
pub async fn delete_scan_item(id: u32, permanent: bool, state: State<'_, ScanState>) -> CmdResult<u64> {
    let (path, size) = with_tree(&state, |tree| {
        let node = tree.node(id)?;
        crate::error::ensure!(id != 0, "The scan root can't be deleted from here");
        Ok((tree.path_of(id), node.size))
    })?;
    crate::error::ensure!(!paths::is_protected(&path), "{} is a protected location", path.display());
    let target = path.clone();
    tauri::async_runtime::spawn_blocking(move || -> anyhow::Result<()> {
        if permanent {
            let meta = fs::symlink_metadata(&target)?;
            if meta.is_dir() {
                fs::remove_dir_all(&target)?;
            } else {
                fs::remove_file(&target)?;
            }
        } else {
            trash::delete(&target).map_err(|e| anyhow::anyhow!("{e}"))?;
        }
        Ok(())
    })
    .await?
    .with_context(|| format!("Couldn't delete {}", path.display()))?;

    if let Some(tree) = state.tree.lock().unwrap().as_mut() {
        tree.remove(id);
    }
    Ok(size)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_and_flattens() {
        let dir = std::env::temp_dir().join(format!("reclaim-scan-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("a/b")).unwrap();
        fs::write(dir.join("a/b/big.bin"), vec![0u8; 4096]).unwrap();
        fs::write(dir.join("a/small.txt"), b"hi").unwrap();
        fs::write(dir.join("root.log"), vec![0u8; 100]).unwrap();

        let ctx = ScanCtx {
            cancel: Arc::new(AtomicBool::new(false)),
            files: AtomicU64::new(0),
            dirs: AtomicU64::new(0),
            bytes: AtomicU64::new(0),
            errors: AtomicU64::new(0),
            current: Mutex::new(String::new()),
        };
        let raw = scan_dir(&dir, "".into(), 0, &ctx);
        let mut tree = build_tree(dir.clone(), raw);
        assert_eq!(tree.nodes[0].size, 4096 + 2 + 100);
        assert_eq!(tree.nodes[0].files, 3);

        // Children are sorted by size: "a" (4098) before "root.log" (100).
        let kids: Vec<_> = tree.children(0).map(|(_, n)| n.name.to_string()).collect();
        assert_eq!(kids, vec!["a", "root.log"]);
        let big = tree.largest_files[0];
        assert_eq!(tree.path_of(big), dir.join("a").join("b").join("big.bin"));

        let a = tree.children(0).next().unwrap().0;
        tree.remove(a);
        assert_eq!(tree.nodes[0].size, 100);
        assert_eq!(tree.nodes[0].files, 1);
        assert_eq!(tree.types.get("bin").unwrap().bytes, 0);
        fs::remove_dir_all(&dir).unwrap();
    }
}
