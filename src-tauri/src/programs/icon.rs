//! Extracts program icons from executables/.ico files and returns them as
//! PNG data URLs the webview can show directly.

use std::collections::HashMap;
use std::sync::Mutex;

use base64::Engine;
use tauri::State;
use windows::Win32::Graphics::Gdi::{
    BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS, DeleteDC, DeleteObject, GetDIBits, HBITMAP,
};
use windows::Win32::UI::WindowsAndMessaging::{DestroyIcon, GetIconInfo, HICON, ICONINFO, PrivateExtractIconsW};
use winreg::RegKey;
use winreg::enums::{HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_64KEY};

use super::{open_entry, parse_icon_ref, split_command};
use crate::error::CmdResult;
use crate::paths::expand_env;

const ICON_SIZE: i32 = 64;

#[derive(Default)]
pub struct IconCache(Mutex<HashMap<String, Option<String>>>);

/// Returns a PNG data URL for the program's icon, or `None` if it has none.
#[tauri::command]
pub async fn program_icon(id: String, cache: State<'_, IconCache>) -> CmdResult<Option<String>> {
    if let Some(hit) = cache.0.lock().unwrap().get(&id) {
        return Ok(hit.clone());
    }
    let lookup_id = id.clone();
    let icon = tauri::async_runtime::spawn_blocking(move || icon_for_program(&lookup_id)).await?;
    cache.0.lock().unwrap().insert(id, icon.clone());
    Ok(icon)
}

fn icon_for_program(id: &str) -> Option<String> {
    let entry = open_entry(id)?;
    let key = &entry.key;
    let mut candidates: Vec<(String, i32)> = Vec::new();
    if let Ok(icon) = key.get_value::<String, _>("DisplayIcon") {
        candidates.push(parse_icon_ref(&icon));
    }
    if let Some(icon) = msi_product_icon(&entry.key_name) {
        candidates.push(parse_icon_ref(&icon));
    }
    if let Ok(cmd) = key.get_value::<String, _>("UninstallString") {
        let (exe, _) = split_command(&cmd);
        let lower = exe.to_lowercase();
        if !lower.ends_with("msiexec.exe") && !lower.ends_with("rundll32.exe") && !lower.eq_ignore_ascii_case("msiexec") {
            candidates.push((exe, 0));
        }
    }
    candidates
        .into_iter()
        .find_map(|(path, index)| extract_png(&expand_env(&path), index))
        .map(|png| format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(png)))
}

/// MSI products often have no DisplayIcon but register a ProductIcon under
/// their "packed" GUID (each GUID segment reversed).
fn msi_product_icon(key_name: &str) -> Option<String> {
    let guid: String = key_name.chars().filter(|c| c.is_ascii_hexdigit()).collect();
    if guid.len() != 32 || !key_name.starts_with('{') {
        return None;
    }
    let rev = |s: &str| s.chars().rev().collect::<String>();
    let mut packed = String::with_capacity(32);
    packed.push_str(&rev(&guid[0..8]));
    packed.push_str(&rev(&guid[8..12]));
    packed.push_str(&rev(&guid[12..16]));
    for pair in guid.as_bytes()[16..].chunks(2) {
        packed.push(pair[1] as char);
        packed.push(pair[0] as char);
    }
    RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey_with_flags(format!(r"SOFTWARE\Classes\Installer\Products\{}", packed.to_uppercase()), KEY_READ | KEY_WOW64_64KEY)
        .and_then(|k| k.get_value::<String, _>("ProductIcon"))
        .ok()
}

pub fn extract_png(path: &str, index: i32) -> Option<Vec<u8>> {
    let wide: Vec<u16> = path.encode_utf16().collect();
    if wide.is_empty() || wide.len() >= 260 {
        return None;
    }
    let mut buf = [0u16; 260];
    buf[..wide.len()].copy_from_slice(&wide);

    let mut icons = [HICON::default()];
    let count = unsafe { PrivateExtractIconsW(&buf, index, ICON_SIZE, ICON_SIZE, Some(&mut icons), None, 0) };
    if count == 0 || count == u32::MAX || icons[0].is_invalid() {
        return None;
    }
    let rgba = unsafe { hicon_to_rgba(icons[0]) };
    unsafe {
        let _ = DestroyIcon(icons[0]);
    }
    let (w, h, pixels) = rgba?;
    encode_png(w, h, &pixels)
}

unsafe fn hicon_to_rgba(icon: HICON) -> Option<(u32, u32, Vec<u8>)> {
    let mut info = ICONINFO::default();
    unsafe { GetIconInfo(icon, &mut info) }.ok()?;
    let result = unsafe { read_bitmaps(info.hbmColor, info.hbmMask) };
    unsafe {
        if !info.hbmColor.is_invalid() {
            let _ = DeleteObject(info.hbmColor.into());
        }
        if !info.hbmMask.is_invalid() {
            let _ = DeleteObject(info.hbmMask.into());
        }
    }
    result
}

unsafe fn read_bitmaps(color: HBITMAP, mask: HBITMAP) -> Option<(u32, u32, Vec<u8>)> {
    if color.is_invalid() {
        return None; // monochrome icon, not worth handling
    }
    let dc = unsafe { CreateCompatibleDC(None) };
    let mut bmi = BITMAPINFO {
        bmiHeader: BITMAPINFOHEADER { biSize: size_of::<BITMAPINFOHEADER>() as u32, ..Default::default() },
        ..Default::default()
    };
    let read = (|| {
        if unsafe { GetDIBits(dc, color, 0, 0, None, &mut bmi, DIB_RGB_COLORS) } == 0 {
            return None;
        }
        let w = bmi.bmiHeader.biWidth;
        let h = bmi.bmiHeader.biHeight.abs();
        if w <= 0 || h <= 0 || w > 512 || h > 512 {
            return None;
        }
        let header = BITMAPINFOHEADER {
            biSize: size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h, // top-down rows
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB.0,
            ..Default::default()
        };
        let mut pixels = vec![0u8; (w * h * 4) as usize];
        let mut bmi32 = BITMAPINFO { bmiHeader: header, ..Default::default() };
        let lines = unsafe { GetDIBits(dc, color, 0, h as u32, Some(pixels.as_mut_ptr().cast()), &mut bmi32, DIB_RGB_COLORS) };
        if lines == 0 {
            return None;
        }
        // Legacy icons carry no alpha channel; derive it from the AND mask.
        if pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) && !mask.is_invalid() {
            let mut mask_px = vec![0u8; (w * h * 4) as usize];
            let mut bmi_mask = BITMAPINFO { bmiHeader: header, ..Default::default() };
            unsafe { GetDIBits(dc, mask, 0, h as u32, Some(mask_px.as_mut_ptr().cast()), &mut bmi_mask, DIB_RGB_COLORS) };
            for (px, m) in pixels.as_chunks_mut::<4>().0.iter_mut().zip(mask_px.as_chunks::<4>().0) {
                px[3] = if m[0] == 0 { 255 } else { 0 };
            }
        }
        for px in pixels.as_chunks_mut::<4>().0 {
            px.swap(0, 2); // BGRA -> RGBA
        }
        Some((w as u32, h as u32, pixels))
    })();
    unsafe {
        let _ = DeleteDC(dc);
    }
    read
}

fn encode_png(w: u32, h: u32, rgba: &[u8]) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut out, w, h);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder.write_header().ok()?;
        writer.write_image_data(rgba).ok()?;
    }
    Some(out)
}

#[cfg(test)]
pub fn __probe_icon(id: &str) -> bool {
    icon_for_program(id).is_some()
}
