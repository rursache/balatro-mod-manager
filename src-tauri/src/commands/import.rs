use std::fs::File;
use std::io::{Cursor, Read, Seek};
use std::path::{Component, Path, PathBuf};

use flate2::read::GzDecoder;
use tar::Archive;
use unrar::Archive as UnrarArchive;
use zip::ZipArchive;

use crate::compat_helper;
use crate::state::AppState;
use bmm_lib::errors::AppError;

async fn sync_compat_helper_after_mod_change(state: &tauri::State<'_, AppState>) {
    let enabled = {
        let db = state.db.lock().unwrap_or_else(|e| e.into_inner());
        db.is_compat_helper_enabled().unwrap_or(false)
    };
    if let Err(err) = compat_helper::sync_compat_helper(enabled) {
        log::warn!("Failed to sync compatibility helper after mod change: {err}");
    }
}

#[tauri::command]
pub async fn process_dropped_file(
    state: tauri::State<'_, AppState>,
    path: String,
) -> Result<String, String> {
    let config_dir =
        dirs::config_dir().ok_or_else(|| "Could not find config directory".to_string())?;
    let mods_dir = config_dir.join("Balatro").join("Mods");
    tokio::fs::create_dir_all(&mods_dir)
        .await
        .map_err(|e| format!("Failed to create mods directory: {e}"))?;

    let file_path_str = path.clone();
    let mods_dir_clone = mods_dir.clone();

    let mod_dir = tokio::task::spawn_blocking(move || {
        let file_path = std::path::Path::new(&file_path_str);
        let file_name = file_path
            .file_name()
            .ok_or_else(|| "Invalid file path".to_string())?
            .to_str()
            .ok_or_else(|| "Invalid file name".to_string())?;

        let file = File::open(file_path).map_err(|e| format!("Failed to open file: {e}"))?;

        extract_archive_from_reader(file_name, file, Some(file_path), &mods_dir_clone)
            .map_err(|e| format!("Failed to extract archive: {e}"))
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))??;

    sync_compat_helper_after_mod_change(&state).await;
    Ok(mod_dir.to_string_lossy().to_string())
}

#[tauri::command]
pub async fn process_mod_archive(
    state: tauri::State<'_, AppState>,
    filename: String,
    data: Vec<u8>,
) -> Result<String, String> {
    let config_dir = dirs::config_dir()
        .ok_or_else(|| AppError::DirNotFound(PathBuf::from("config directory")).to_string())?;
    let mods_dir = config_dir.join("Balatro").join("Mods");
    tokio::fs::create_dir_all(&mods_dir)
        .await
        .map_err(|e| format!("Failed to create mods directory: {e}"))?;

    let mods_dir_clone = mods_dir.clone();
    let filename_clone = filename.clone();

    let mod_dir = tokio::task::spawn_blocking(move || {
        let cursor = Cursor::new(data);
        let mod_dir = extract_archive_from_reader(&filename_clone, cursor, None, &mods_dir_clone)?;

        // Flatten nested directory if needed
        if let Ok(entries) = std::fs::read_dir(&mod_dir) {
            let dirs: Vec<_> = entries
                .filter_map(Result::ok)
                .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
                .collect();
            if dirs.len() == 1 && std::fs::read_dir(&mod_dir).map(|e| e.count()).unwrap_or(0) == 1 {
                let nested_dir = dirs[0].path();
                for entry in std::fs::read_dir(&nested_dir)
                    .map_err(|e| format!("Failed to read nested directory: {e}"))?
                {
                    let entry = entry.map_err(|e| format!("Failed to read entry: {e}"))?;
                    let target_path = mod_dir.join(entry.file_name());
                    if entry
                        .file_type()
                        .map_err(|e| format!("Failed to get file type: {e}"))?
                        .is_dir()
                    {
                        std::fs::rename(entry.path(), &target_path)
                            .map_err(|e| format!("Failed to move directory: {e}"))?;
                    } else {
                        std::fs::rename(entry.path(), &target_path)
                            .map_err(|e| format!("Failed to move file: {e}"))?;
                    }
                }
                std::fs::remove_dir_all(&nested_dir)
                    .map_err(|e| format!("Failed to remove nested directory: {e}"))?;
            }
        }

        Ok::<_, String>(mod_dir)
    })
    .await
    .map_err(|e| format!("Task join error: {e}"))??;

    sync_compat_helper_after_mod_change(&state).await;
    Ok(mod_dir.to_string_lossy().to_string())
}

fn extract_archive_from_reader<R: Read + Seek>(
    filename: &str,
    reader: R,
    source_path: Option<&Path>,
    mods_dir: &std::path::Path,
) -> Result<PathBuf, String> {
    let mod_dir_name = filename
        .trim_end_matches(".zip")
        .trim_end_matches(".tar.gz")
        .trim_end_matches(".tgz")
        .trim_end_matches(".tar")
        .trim_end_matches(".rar")
        .trim_end_matches(".7z")
        .to_string();
    let mod_dir = mods_dir.join(&mod_dir_name);

    if mod_dir.exists() {
        std::fs::remove_dir_all(&mod_dir)
            .map_err(|e| format!("Failed to remove existing mod directory: {e}"))?;
    }

    if filename.ends_with(".zip") {
        extract_zip(reader, &mod_dir)?;
    } else if filename.ends_with(".rar") {
        let source_path =
            source_path.ok_or_else(|| "RAR archives require a file path".to_string())?;
        extract_rar(source_path, &mod_dir)?;
    } else if filename.ends_with(".7z") {
        extract_7z(reader, &mod_dir)?;
    } else if filename.ends_with(".tar") {
        extract_tar(reader, &mod_dir)?;
    } else if filename.ends_with(".tar.gz") || filename.ends_with(".tgz") {
        extract_tar_gz(reader, &mod_dir)?;
    } else {
        return Err(
            "Unsupported file format. Only ZIP, TAR, TAR.GZ, RAR, and 7Z are supported."
                .to_string(),
        );
    }

    Ok(mod_dir)
}

fn is_junk_entry(path: &Path) -> bool {
    path.components().any(|c| c.as_os_str() == "__MACOSX")
        || path.file_name().is_some_and(|n| n == ".DS_Store")
}

// Relative paths made only of normal components cannot resolve outside the
// extraction root, even when following other links that obey the same rule
fn is_contained_relative_path(path: &Path) -> bool {
    let mut has_normal = false;
    for component in path.components() {
        match component {
            Component::Normal(_) => has_normal = true,
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return false,
        }
    }
    has_normal
}

/// Extracts a 7Z archive.
///
/// Entry paths are validated by `sevenz-rust2`, which rejects any name that would
/// escape `target_dir` (`..`, absolute paths, or a Windows drive prefix) on every
/// platform. See `test_extract_7z_rejects_path_traversal`.
fn extract_7z<R: Read + Seek>(reader: R, target_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create target directory: {e}"))?;

    sevenz_rust2::decompress(reader, target_dir)
        .map_err(|e| format!("Failed to extract 7Z archive: {e}"))
}

fn extract_zip<R: Read + Seek>(reader: R, target_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create target directory: {e}"))?;
    let mut archive =
        ZipArchive::new(reader).map_err(|e| format!("Failed to open ZIP archive: {e}"))?;
    for i in 0..archive.len() {
        let mut file = archive
            .by_index(i)
            .map_err(|e| format!("Failed to access file in archive: {e}"))?;
        let file_path = match file.enclosed_name() {
            Some(path) => path,
            None => {
                log::warn!("Skipping unsafe ZIP entry: {}", file.name());
                continue;
            }
        };
        if is_junk_entry(&file_path) {
            continue;
        }
        let output_path = target_dir.join(&file_path);
        if file.is_dir() {
            std::fs::create_dir_all(&output_path)
                .map_err(|e| format!("Failed to create directory: {e}"))?;
        } else {
            if let Some(parent) = output_path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|e| format!("Failed to create parent directory: {e}"))?;
            }
            let mut outfile = std::fs::File::create(&output_path)
                .map_err(|e| format!("Failed to create file {}: {e}", output_path.display()))?;
            std::io::copy(&mut file, &mut outfile)
                .map_err(|e| format!("Failed to write file {}: {e}", output_path.display()))?;
        }
    }
    Ok(())
}

fn extract_tar<R: Read>(reader: R, target_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create target directory: {e}"))?;
    let mut archive = Archive::new(reader);
    for entry in archive
        .entries()
        .map_err(|e| format!("Failed to read TAR entries: {e}"))?
    {
        let mut entry = entry.map_err(|e| format!("Failed to read TAR entry: {e}"))?;
        let path = entry
            .path()
            .map_err(|e| format!("Failed to get entry path: {e}"))?
            .into_owned();
        if is_junk_entry(&path) {
            continue;
        }

        let kind = entry.header().entry_type();
        if kind.is_symlink() || kind.is_hard_link() {
            let link_target = entry
                .link_name()
                .map_err(|e| format!("Failed to read link target of {}: {e}", path.display()))?;
            if !link_target
                .as_deref()
                .is_some_and(is_contained_relative_path)
            {
                log::warn!(
                    "Skipping TAR link escaping the mod directory: {}",
                    path.display()
                );
                continue;
            }
        }

        // unpack_in skips entries with `..`, strips leading `/` and refuses to
        // write through links that resolve outside target_dir
        let unpacked = entry
            .unpack_in(target_dir)
            .map_err(|e| format!("Failed to unpack {}: {e}", path.display()))?;
        if !unpacked {
            log::warn!("Skipping unsafe TAR entry: {}", path.display());
        }
    }
    Ok(())
}

fn extract_tar_gz<R: Read>(reader: R, target_dir: &Path) -> Result<(), String> {
    extract_tar(GzDecoder::new(reader), target_dir)
}

fn extract_rar(path: &Path, target_dir: &Path) -> Result<(), String> {
    std::fs::create_dir_all(target_dir)
        .map_err(|e| format!("Failed to create target directory: {e}"))?;
    let mut archive = UnrarArchive::new(path)
        .as_first_part()
        .open_for_processing()
        .map_err(|e| format!("Failed to open RAR archive: {e}"))?;
    while let Some(header) = archive
        .read_header()
        .map_err(|e| format!("Failed to read RAR entry: {e}"))?
    {
        let entry = header.entry();
        let safe = is_contained_relative_path(&entry.filename);
        if !safe {
            log::warn!("Skipping unsafe RAR entry: {}", entry.filename.display());
        }
        let extract = safe && entry.is_file() && !is_junk_entry(&entry.filename);
        archive = if extract {
            header
                .extract_with_base(target_dir)
                .map_err(|e| format!("Failed to extract RAR entry: {e}"))?
        } else {
            header
                .skip()
                .map_err(|e| format!("Failed to skip RAR entry: {e}"))?
        };
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    #[test]
    fn test_extract_zip_basic() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        // Create a minimal valid ZIP file in memory
        let mut zip_data = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_data);
            let mut zip = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("test.txt", options).unwrap();
            zip.write_all(b"hello world").unwrap();
            zip.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(zip_data);
        extract_zip(cursor, &target).unwrap();

        assert!(target.join("test.txt").exists());
        let content = std::fs::read_to_string(target.join("test.txt")).unwrap();
        assert_eq!(content, "hello world");
    }

    #[test]
    fn test_extract_zip_with_directory() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        let mut zip_data = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_data);
            let mut zip = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            zip.add_directory("subdir/", options).unwrap();
            zip.start_file("subdir/file.txt", options).unwrap();
            zip.write_all(b"nested content").unwrap();
            zip.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(zip_data);
        extract_zip(cursor, &target).unwrap();

        assert!(target.join("subdir").is_dir());
        assert!(target.join("subdir/file.txt").exists());
    }

    #[test]
    fn test_extract_zip_skips_macosx_folder() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        let mut zip_data = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_data);
            let mut zip = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("valid.txt", options).unwrap();
            zip.write_all(b"valid").unwrap();
            zip.start_file("__MACOSX/junk.txt", options).unwrap();
            zip.write_all(b"junk").unwrap();
            zip.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(zip_data);
        extract_zip(cursor, &target).unwrap();

        assert!(target.join("valid.txt").exists());
        assert!(!target.join("__MACOSX").exists());
    }

    #[test]
    fn test_extract_tar_basic() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        let mut tar_data = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_data);
            let mut header = tar::Header::new_gnu();
            let content = b"tar content";
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "file.txt", &content[..])
                .unwrap();
            builder.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(tar_data);
        extract_tar(cursor, &target).unwrap();

        assert!(target.join("file.txt").exists());
    }

    #[test]
    fn test_extract_tar_gz_basic() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        // Create tar first
        let mut tar_data = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut tar_data);
            let mut header = tar::Header::new_gnu();
            let content = b"gzipped tar content";
            header.set_size(content.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder
                .append_data(&mut header, "gzfile.txt", &content[..])
                .unwrap();
            builder.finish().unwrap();
        }

        // Compress with gzip
        let mut gz_data = Vec::new();
        {
            let mut encoder =
                flate2::write::GzEncoder::new(&mut gz_data, flate2::Compression::default());
            encoder.write_all(&tar_data).unwrap();
            encoder.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(gz_data);
        extract_tar_gz(cursor, &target).unwrap();

        assert!(target.join("gzfile.txt").exists());
    }

    enum RawEntry<'a> {
        File(&'a str, &'a [u8]),
        Symlink(&'a str, &'a str),
        HardLink(&'a str, &'a str),
    }

    fn append_long_name(
        builder: &mut tar::Builder<&mut Vec<u8>>,
        kind: tar::EntryType,
        name: &str,
    ) {
        let mut data = name.as_bytes().to_vec();
        data.push(0);
        let mut header = tar::Header::new_gnu();
        header.as_old_mut().name[..13].copy_from_slice(b"././@LongLink");
        header.set_entry_type(kind);
        header.set_mode(0o644);
        header.set_size(data.len() as u64);
        header.set_cksum();
        builder.append(&header, &data[..]).unwrap();
    }

    // Writes header bytes verbatim so `..` and absolute names reach the archive
    fn raw_tar(entries: &[RawEntry]) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut builder = tar::Builder::new(&mut out);
            for entry in entries {
                let (path, kind, link, data): (&str, _, Option<&str>, &[u8]) = match entry {
                    RawEntry::File(p, d) => (p, tar::EntryType::Regular, None, d),
                    RawEntry::Symlink(p, l) => (p, tar::EntryType::Symlink, Some(l), &[]),
                    RawEntry::HardLink(p, l) => (p, tar::EntryType::Link, Some(l), &[]),
                };
                if path.len() >= 100 {
                    append_long_name(&mut builder, tar::EntryType::GNULongName, path);
                }
                if let Some(link) = link.filter(|l| l.len() >= 100) {
                    append_long_name(&mut builder, tar::EntryType::GNULongLink, link);
                }
                let mut header = tar::Header::new_gnu();
                let name = &path.as_bytes()[..path.len().min(99)];
                header.as_old_mut().name[..name.len()].copy_from_slice(name);
                if let Some(link) = link {
                    let link = &link.as_bytes()[..link.len().min(99)];
                    header.set_link_name_literal(link).unwrap();
                }
                header.set_entry_type(kind);
                header.set_mode(0o644);
                header.set_size(data.len() as u64);
                header.set_cksum();
                builder.append(&header, data).unwrap();
            }
            builder.finish().unwrap();
        }
        out
    }

    fn gzip(data: &[u8]) -> Vec<u8> {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        encoder.write_all(data).unwrap();
        encoder.finish().unwrap()
    }

    fn traversal_tar() -> Vec<u8> {
        raw_tar(&[
            RawEntry::File("../escape.txt", b"pwned"),
            RawEntry::File("nested/../../escape2.txt", b"pwned"),
            RawEntry::File("ok.txt", b"ok"),
        ])
    }

    fn assert_no_traversal(root: &Path, target: &Path) {
        assert!(target.join("ok.txt").exists());
        assert!(!root.join("escape.txt").exists());
        assert!(!root.join("escape2.txt").exists());
        assert!(!target.join("escape2.txt").exists());
    }

    #[test]
    fn test_extract_tar_skips_parent_dir_entries() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        extract_tar(Cursor::new(traversal_tar()), &target).unwrap();

        assert_no_traversal(temp_dir.path(), &target);
    }

    #[test]
    fn test_extract_tar_gz_skips_parent_dir_entries() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        extract_tar_gz(Cursor::new(gzip(&traversal_tar())), &target).unwrap();

        assert_no_traversal(temp_dir.path(), &target);
    }

    fn absolute_tar(outside: &Path) -> (Vec<u8>, PathBuf) {
        let abs = outside.join("abs.txt");
        let tar = raw_tar(&[RawEntry::File(abs.to_str().unwrap(), b"pwned")]);
        (tar, abs)
    }

    fn assert_absolute_entry_contained(target: &Path, abs: &Path) {
        assert!(!abs.exists());
        let relative: PathBuf = abs
            .components()
            .filter(|c| matches!(c, Component::Normal(_)))
            .collect();
        assert!(target.join(relative).exists());
    }

    #[test]
    fn test_extract_tar_keeps_absolute_entries_inside_target() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let (tar, abs) = absolute_tar(&outside);

        extract_tar(Cursor::new(tar), &target).unwrap();

        assert_absolute_entry_contained(&target, &abs);
    }

    #[test]
    fn test_extract_tar_gz_keeps_absolute_entries_inside_target() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        let (tar, abs) = absolute_tar(&outside);

        extract_tar_gz(Cursor::new(gzip(&tar)), &target).unwrap();

        assert_absolute_entry_contained(&target, &abs);
    }

    fn link_escape_tar(outside: &Path) -> Vec<u8> {
        let outside_abs = outside.to_str().unwrap();
        raw_tar(&[
            RawEntry::Symlink("rel", "../outside"),
            RawEntry::Symlink("abs", outside_abs),
            RawEntry::HardLink("hard", "../outside/secret.txt"),
            RawEntry::File("rel/pwned.txt", b"pwned"),
            RawEntry::File("abs/pwned.txt", b"pwned"),
        ])
    }

    fn assert_no_link_escape(target: &Path, outside: &Path) {
        for name in ["rel", "abs"] {
            let meta = std::fs::symlink_metadata(target.join(name)).unwrap();
            assert!(!meta.file_type().is_symlink(), "{name} must not be a link");
        }
        assert!(!target.join("hard").exists());
        assert!(!outside.join("pwned.txt").exists());
        assert_eq!(
            std::fs::read_to_string(outside.join("secret.txt")).unwrap(),
            "secret"
        );
    }

    #[test]
    fn test_extract_tar_skips_escaping_links() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();

        extract_tar(Cursor::new(link_escape_tar(&outside)), &target).unwrap();

        assert_no_link_escape(&target, &outside);
    }

    #[test]
    fn test_extract_tar_gz_skips_escaping_links() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");
        let outside = temp_dir.path().join("outside");
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret.txt"), "secret").unwrap();

        extract_tar_gz(Cursor::new(gzip(&link_escape_tar(&outside))), &target).unwrap();

        assert_no_link_escape(&target, &outside);
    }

    #[cfg(unix)]
    #[test]
    fn test_extract_tar_keeps_contained_symlink() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");
        let tar = raw_tar(&[
            RawEntry::File("main.lua", b"-- mod"),
            RawEntry::Symlink("alias.lua", "main.lua"),
        ]);

        extract_tar(Cursor::new(tar), &target).unwrap();

        let meta = std::fs::symlink_metadata(target.join("alias.lua")).unwrap();
        assert!(meta.file_type().is_symlink());
        assert_eq!(
            std::fs::read_to_string(target.join("alias.lua")).unwrap(),
            "-- mod"
        );
    }

    #[test]
    fn test_extract_tar_skips_macos_junk() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");
        let tar = raw_tar(&[
            RawEntry::File("__MACOSX/._main.lua", b"junk"),
            RawEntry::File(".DS_Store", b"junk"),
            RawEntry::File("sub/.DS_Store", b"junk"),
            RawEntry::File("sub/main.lua", b"-- mod"),
        ]);

        extract_tar(Cursor::new(tar), &target).unwrap();

        assert!(target.join("sub/main.lua").exists());
        assert!(!target.join("__MACOSX").exists());
        assert!(!target.join(".DS_Store").exists());
        assert!(!target.join("sub/.DS_Store").exists());
    }

    #[test]
    fn test_extract_zip_skips_ds_store_and_traversal() {
        let temp_dir = TempDir::new().unwrap();
        let target = temp_dir.path().join("extracted");

        let mut zip_data = Vec::new();
        {
            let mut zip = zip::ZipWriter::new(Cursor::new(&mut zip_data));
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("sub/.DS_Store", options).unwrap();
            zip.write_all(b"junk").unwrap();
            zip.start_file("../escape.txt", options).unwrap();
            zip.write_all(b"pwned").unwrap();
            zip.start_file("sub/main.lua", options).unwrap();
            zip.write_all(b"-- mod").unwrap();
            zip.finish().unwrap();
        }

        extract_zip(Cursor::new(zip_data), &target).unwrap();

        assert!(target.join("sub/main.lua").exists());
        assert!(!target.join("sub/.DS_Store").exists());
        assert!(!temp_dir.path().join("escape.txt").exists());
    }

    #[test]
    fn test_is_contained_relative_path() {
        assert!(is_contained_relative_path(Path::new("a/b.lua")));
        assert!(is_contained_relative_path(Path::new("./a")));
        assert!(!is_contained_relative_path(Path::new("../a")));
        assert!(!is_contained_relative_path(Path::new("a/../../b")));
        assert!(!is_contained_relative_path(Path::new("/etc/passwd")));
        assert!(!is_contained_relative_path(Path::new("")));
        assert!(!is_contained_relative_path(Path::new(".")));
    }

    #[test]
    fn test_mod_dir_name_from_zip() {
        let filename = "TestMod.zip";
        let mod_name = filename
            .trim_end_matches(".zip")
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".tgz")
            .trim_end_matches(".tar")
            .trim_end_matches(".rar");
        assert_eq!(mod_name, "TestMod");
    }

    #[test]
    fn test_mod_dir_name_from_tar_gz() {
        let filename = "TestMod.tar.gz";
        let mod_name = filename
            .trim_end_matches(".zip")
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".tgz")
            .trim_end_matches(".tar")
            .trim_end_matches(".rar");
        assert_eq!(mod_name, "TestMod");
    }

    #[test]
    fn test_mod_dir_name_from_tgz() {
        let filename = "TestMod.tgz";
        let mod_name = filename
            .trim_end_matches(".zip")
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".tgz")
            .trim_end_matches(".tar")
            .trim_end_matches(".rar");
        assert_eq!(mod_name, "TestMod");
    }

    #[test]
    fn test_mod_dir_name_from_rar() {
        let filename = "TestMod.rar";
        let mod_name = filename
            .trim_end_matches(".zip")
            .trim_end_matches(".tar.gz")
            .trim_end_matches(".tgz")
            .trim_end_matches(".tar")
            .trim_end_matches(".rar");
        assert_eq!(mod_name, "TestMod");
    }

    #[test]
    fn test_extract_7z_basic() {
        let temp_dir = TempDir::new().unwrap();
        let source_dir = temp_dir.path().join("source");
        std::fs::create_dir_all(&source_dir).unwrap();
        std::fs::write(source_dir.join("init.lua"), "-- mod").unwrap();

        std::fs::create_dir_all(source_dir.join("assets")).unwrap();
        std::fs::write(source_dir.join("assets/card.png"), "png").unwrap();

        let archive_path = temp_dir.path().join("mod.7z");
        let mut archive = sevenz_rust2::ArchiveWriter::create(&archive_path).unwrap();
        archive.push_source_path(&source_dir, |_| true).unwrap();
        archive.finish().unwrap();

        let target = temp_dir.path().join("extracted");
        extract_7z(File::open(&archive_path).unwrap(), &target).unwrap();

        assert_eq!(
            std::fs::read_to_string(target.join("init.lua")).unwrap(),
            "-- mod"
        );
        assert_eq!(
            std::fs::read_to_string(target.join("assets/card.png")).unwrap(),
            "png"
        );
    }

    #[test]
    fn test_extract_7z_rejects_path_traversal() {
        let temp_dir = TempDir::new().unwrap();

        // Hand-build an archive whose entry name escapes the extraction directory.
        let archive_path = temp_dir.path().join("evil.7z");
        let mut archive = sevenz_rust2::ArchiveWriter::create(&archive_path).unwrap();
        let entry = sevenz_rust2::ArchiveEntry::new_file("../escaped.lua");
        archive
            .push_archive_entry(entry, Some(Cursor::new(b"pwned".to_vec())))
            .unwrap();
        archive.finish().unwrap();

        let target = temp_dir.path().join("extracted");
        let result = extract_7z(File::open(&archive_path).unwrap(), &target);

        assert!(result.is_err(), "traversal entry should be rejected");
        assert!(
            !temp_dir.path().join("escaped.lua").exists(),
            "archive escaped the extraction directory"
        );
    }

    #[test]
    fn test_unsupported_format_error() {
        let temp_dir = TempDir::new().unwrap();
        let mods_dir = temp_dir.path();
        let cursor = std::io::Cursor::new(Vec::<u8>::new());

        let result = extract_archive_from_reader("file.invalid", cursor, None, mods_dir);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported file format"));
    }

    #[test]
    fn test_rar_requires_file_path() {
        let temp_dir = TempDir::new().unwrap();
        let mods_dir = temp_dir.path();
        let cursor = std::io::Cursor::new(Vec::<u8>::new());

        let result = extract_archive_from_reader("file.rar", cursor, None, mods_dir);
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("RAR archives require a file path")
        );
    }

    #[test]
    fn test_extract_archive_creates_mod_dir() {
        let temp_dir = TempDir::new().unwrap();
        let mods_dir = temp_dir.path();

        let mut zip_data = Vec::new();
        {
            let cursor = std::io::Cursor::new(&mut zip_data);
            let mut zip = zip::ZipWriter::new(cursor);
            let options = zip::write::SimpleFileOptions::default();
            zip.start_file("init.lua", options).unwrap();
            zip.write_all(b"-- mod").unwrap();
            zip.finish().unwrap();
        }

        let cursor = std::io::Cursor::new(zip_data);
        let result = extract_archive_from_reader("MyMod.zip", cursor, None, mods_dir);

        assert!(result.is_ok());
        let mod_dir = result.unwrap();
        assert_eq!(mod_dir.file_name().unwrap().to_str().unwrap(), "MyMod");
        assert!(mod_dir.join("init.lua").exists());
    }
}
