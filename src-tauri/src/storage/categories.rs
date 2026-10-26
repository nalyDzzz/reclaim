/// Groups file extensions into the categories shown in the storage view.
pub fn category_for(ext: &str) -> &'static str {
    match ext {
        "mp4" | "mkv" | "mov" | "avi" | "wmv" | "webm" | "m4v" | "flv" | "mpg" | "mpeg" | "m2ts" | "3gp" => "video",
        "jpg" | "jpeg" | "png" | "gif" | "bmp" | "webp" | "heic" | "heif" | "tif" | "tiff" | "raw" | "cr2" | "cr3" | "nef" | "arw"
        | "dng" | "psd" | "svg" | "ico" | "avif" => "images",
        "mp3" | "flac" | "wav" | "aac" | "ogg" | "m4a" | "wma" | "opus" | "aiff" | "alac" => "audio",
        "zip" | "7z" | "rar" | "tar" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "cab" | "lz4" => "archives",
        "iso" | "img" | "vhd" | "vhdx" | "vmdk" | "vdi" | "qcow2" | "wim" | "esd" => "diskImages",
        "exe" | "dll" | "sys" | "msi" | "msix" | "appx" | "msp" | "ocx" | "drv" | "efi" | "mui" | "node" | "so" | "dylib" => "programs",
        "pdf" | "doc" | "docx" | "xls" | "xlsx" | "ppt" | "pptx" | "odt" | "ods" | "odp" | "txt" | "rtf" | "md" | "csv" | "epub"
        | "one" | "pst" | "ost" => "documents",
        "pak" | "upk" | "uasset" | "umap" | "bsa" | "ba2" | "vpk" | "bundle" | "assets" | "resource" | "big" | "forge" | "pck" | "rpf"
        | "sb" | "toc" | "cas" | "wad" | "ucas" | "utoc" => "gameData",
        "js" | "ts" | "tsx" | "jsx" | "mjs" | "cjs" | "json" | "rs" | "py" | "pyc" | "java" | "class" | "jar" | "go" | "c" | "h"
        | "cpp" | "hpp" | "cs" | "rlib" | "rmeta" | "pdb" | "obj" | "o" | "lib" | "a" | "wasm" | "map" | "lock" | "d" => "code",
        "log" | "etl" | "dmp" | "tmp" | "temp" | "cache" | "bak" | "old" | "evtx" | "nvph" => "logsAndTemp",
        "db" | "sqlite" | "sqlite3" | "ldb" | "mdb" | "accdb" | "edb" | "dat" | "blob" => "data",
        _ => "other",
    }
}
