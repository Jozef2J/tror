use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Format {
    Zip,
    TarGz,
    SevenZip,
    Rar,
}

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Zip => "ZIP",
            Format::TarGz => "TAR.GZ",
            Format::SevenZip => "7z (7-Zip)",
            Format::Rar => "RAR (WinRAR)",
        }
    }

    fn ext(self) -> &'static str {
        match self {
            Format::Zip => ".zip",
            Format::TarGz => ".tar.gz",
            Format::SevenZip => ".7z",
            Format::Rar => ".rar",
        }
    }
}

fn seven_zip() -> Option<PathBuf> {
    let mut found: Vec<PathBuf> = [
        "C:\\Program Files\\7-Zip\\7z.exe",
        "C:\\Program Files (x86)\\7-Zip\\7z.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .filter(|p| cfg!(windows) && p.is_file())
    .collect();
    found.extend(which("7z"));
    found.into_iter().next()
}

fn winrar() -> Option<PathBuf> {
    [
        "C:\\Program Files\\WinRAR\\Rar.exe",
        "C:\\Program Files (x86)\\WinRAR\\Rar.exe",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|p| cfg!(windows) && p.is_file())
    .or_else(|| which("rar"))
}

fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .flat_map(|dir| [dir.join(name), dir.join(format!("{name}.exe"))])
        .find(|p| p.is_file())
}

pub fn formats() -> Vec<Format> {
    let mut list = vec![Format::Zip, Format::TarGz];
    if seven_zip().is_some() {
        list.push(Format::SevenZip);
    }
    if winrar().is_some() {
        list.push(Format::Rar);
    }
    list
}

fn target(paths: &[PathBuf], format: Format) -> Result<PathBuf, String> {
    let first = paths.first().ok_or("нечего архивировать")?;
    let dir = first.parent().ok_or("нет родительской папки")?;
    let base = if paths.len() == 1 {
        first
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "архив".into())
    } else {
        "архив".into()
    };
    let ext = format.ext();
    let first_try = dir.join(format!("{base}{ext}"));
    if !first_try.exists() {
        return Ok(first_try);
    }
    (2..)
        .map(|n| dir.join(format!("{base} ({n}){ext}")))
        .find(|p| !p.exists())
        .ok_or_else(|| "не удалось подобрать имя".into())
}

fn walk(root: &Path, base: &Path, out: &mut Vec<(PathBuf, String, bool)>) -> std::io::Result<()> {
    let name = root
        .strip_prefix(base)
        .unwrap_or(root)
        .components()
        .map(|c| c.as_os_str().to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("/");
    let meta = std::fs::symlink_metadata(root)?;
    if meta.is_dir() {
        out.push((root.to_path_buf(), format!("{name}/"), true));
        let mut entries: Vec<PathBuf> = std::fs::read_dir(root)?
            .filter_map(|e| e.ok().map(|e| e.path()))
            .collect();
        entries.sort();
        for entry in entries {
            walk(&entry, base, out)?;
        }
    } else if meta.is_file() {
        out.push((root.to_path_buf(), name, false));
    }
    Ok(())
}

fn entries(paths: &[PathBuf]) -> Result<Vec<(PathBuf, String, bool)>, String> {
    let mut out = Vec::new();
    for path in paths {
        let base = path.parent().unwrap_or(Path::new(""));
        walk(path, base, &mut out).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    Ok(out)
}

fn zip(paths: &[PathBuf], to: &Path) -> Result<(), String> {
    let items = entries(paths)?;
    let mut file = std::io::BufWriter::new(std::fs::File::create(to).map_err(|e| e.to_string())?);
    let mut central = Vec::new();
    let mut offset: u64 = 0;
    let mut count = 0u16;
    for (path, name, dir) in items {
        let data = if dir {
            Vec::new()
        } else {
            std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?
        };
        let mut crc = flate2::Crc::new();
        crc.update(&data);
        let (method, packed) = if data.is_empty() {
            (0u16, Vec::new())
        } else {
            let mut encoder =
                flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
            encoder.write_all(&data).map_err(|e| e.to_string())?;
            (8u16, encoder.finish().map_err(|e| e.to_string())?)
        };
        if data.len() as u64 >= u32::MAX as u64 || offset >= u32::MAX as u64 {
            return Err("ZIP больше 4 ГБ не поддерживается — выберите TAR.GZ".into());
        }
        if count == u16::MAX {
            return Err("слишком много файлов для ZIP — выберите TAR.GZ".into());
        }
        let name_bytes = name.as_bytes();
        let header = |sig: u32, central_dir: bool| {
            let mut h = Vec::new();
            h.extend_from_slice(&sig.to_le_bytes());
            if central_dir {
                h.extend_from_slice(&20u16.to_le_bytes());
            }
            h.extend_from_slice(&20u16.to_le_bytes());
            h.extend_from_slice(&0x0800u16.to_le_bytes());
            h.extend_from_slice(&method.to_le_bytes());
            h.extend_from_slice(&0u32.to_le_bytes());
            h.extend_from_slice(&crc.sum().to_le_bytes());
            h.extend_from_slice(&(packed.len() as u32).to_le_bytes());
            h.extend_from_slice(&(data.len() as u32).to_le_bytes());
            h.extend_from_slice(&(name_bytes.len() as u16).to_le_bytes());
            h.extend_from_slice(&0u16.to_le_bytes());
            h
        };
        let local = header(0x0403_4b50, false);
        file.write_all(&local).map_err(|e| e.to_string())?;
        file.write_all(name_bytes).map_err(|e| e.to_string())?;
        file.write_all(&packed).map_err(|e| e.to_string())?;
        let mut entry = header(0x0201_4b50, true);
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&0u16.to_le_bytes());
        entry.extend_from_slice(&(if dir { 0x10u32 } else { 0 }).to_le_bytes());
        entry.extend_from_slice(&(offset as u32).to_le_bytes());
        entry.extend_from_slice(name_bytes);
        central.push(entry);
        offset += (local.len() + name_bytes.len() + packed.len()) as u64;
        count += 1;
    }
    let size: usize = central.iter().map(Vec::len).sum();
    for entry in &central {
        file.write_all(entry).map_err(|e| e.to_string())?;
    }
    let mut end = Vec::new();
    end.extend_from_slice(&0x0605_4b50u32.to_le_bytes());
    end.extend_from_slice(&[0; 4]);
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&count.to_le_bytes());
    end.extend_from_slice(&(size as u32).to_le_bytes());
    end.extend_from_slice(&(offset as u32).to_le_bytes());
    end.extend_from_slice(&0u16.to_le_bytes());
    file.write_all(&end).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())
}

fn tar_header(name: &str, size: u64, dir: bool, kind: u8) -> [u8; 512] {
    let mut h = [0u8; 512];
    let bytes = name.as_bytes();
    let len = bytes.len().min(100);
    h[..len].copy_from_slice(&bytes[..len]);
    let octal = |field: &mut [u8], value: u64| {
        let text = format!("{:0width$o}", value, width = field.len() - 1);
        field[..text.len()].copy_from_slice(text.as_bytes());
    };
    octal(&mut h[100..108], if dir { 0o755 } else { 0o644 });
    octal(&mut h[108..116], 0);
    octal(&mut h[116..124], 0);
    octal(&mut h[124..136], size);
    let mtime = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    octal(&mut h[136..148], mtime);
    h[148..156].copy_from_slice(b"        ");
    h[156] = kind;
    h[257..263].copy_from_slice(b"ustar ");
    h[263..265].copy_from_slice(b" \0");
    let sum: u32 = h.iter().map(|b| *b as u32).sum();
    let text = format!("{sum:06o}\0 ");
    h[148..156].copy_from_slice(text.as_bytes());
    h
}

fn tar_gz(paths: &[PathBuf], to: &Path) -> Result<(), String> {
    let items = entries(paths)?;
    let file = std::fs::File::create(to).map_err(|e| e.to_string())?;
    let mut out = flate2::write::GzEncoder::new(
        std::io::BufWriter::new(file),
        flate2::Compression::default(),
    );
    let err = |e: std::io::Error| e.to_string();
    for (path, name, dir) in items {
        if name.len() > 100 {
            let long = format!("{name}\0");
            out.write_all(&tar_header("././@LongLink", long.len() as u64, false, b'L'))
                .map_err(err)?;
            out.write_all(long.as_bytes()).map_err(err)?;
            out.write_all(&vec![0u8; (512 - long.len() % 512) % 512])
                .map_err(err)?;
        }
        if dir {
            out.write_all(&tar_header(&name, 0, true, b'5'))
                .map_err(err)?;
            continue;
        }
        let size = std::fs::metadata(&path).map_err(err)?.len();
        out.write_all(&tar_header(&name, size, false, b'0'))
            .map_err(err)?;
        let mut source =
            std::fs::File::open(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let copied = std::io::copy(&mut source, &mut out).map_err(err)?;
        if copied != size {
            return Err(format!(
                "{}: файл изменился во время архивирования",
                path.display()
            ));
        }
        out.write_all(&vec![0u8; ((512 - size % 512) % 512) as usize])
            .map_err(err)?;
    }
    out.write_all(&[0u8; 1024]).map_err(err)?;
    out.finish().map_err(err)?.flush().map_err(err)
}

fn external(tool: PathBuf, args: &[&str], paths: &[PathBuf], to: &Path) -> Result<(), String> {
    let dir = paths[0].parent().ok_or("нет родительской папки")?;
    let mut command = Command::new(tool);
    command.current_dir(dir).args(args).arg(to);
    for path in paths {
        command.arg(path.file_name().ok_or("пустое имя")?);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000);
    }
    let output = command.output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        let text = String::from_utf8_lossy(&output.stderr).trim().to_owned();
        Err(if text.is_empty() {
            format!("код завершения {}", output.status)
        } else {
            text
        })
    }
}

pub fn create(paths: &[PathBuf], format: Format) -> Result<PathBuf, String> {
    let to = target(paths, format)?;
    let result = match format {
        Format::Zip => zip(paths, &to),
        Format::TarGz => tar_gz(paths, &to),
        Format::SevenZip => external(
            seven_zip().ok_or("7-Zip не найден")?,
            &["a", "-y"],
            paths,
            &to,
        ),
        Format::Rar => external(
            winrar().ok_or("WinRAR не найден")?,
            &["a", "-r", "-y"],
            paths,
            &to,
        ),
    };
    if result.is_err() {
        let _ = std::fs::remove_file(&to);
    }
    result.map(|_| to)
}

pub fn remote_command(dir: &str, names: &[String], zip: bool) -> String {
    let quote = crate::remote::quote;
    let base = if names.len() == 1 {
        names[0].clone()
    } else {
        "архив".into()
    };
    let ext = if zip { ".zip" } else { ".tar.gz" };
    let list: Vec<String> = names.iter().map(|n| quote(n)).collect();
    let list = list.join(" ");
    let pack = if zip {
        format!(
            "if command -v zip >/dev/null 2>&1; then zip -rqy \"$f\" {list}; elif command -v python3 >/dev/null 2>&1; then python3 -m zipfile -c \"$f\" {list}; else bsdtar -a -cf \"$f\" {list}; fi"
        )
    } else {
        format!("tar -czf \"$f\" -- {list}")
    };
    format!(
        "cd {} && n={} && f=\"$n{ext}\" && i=2 && while [ -e \"$f\" ]; do f=\"$n ($i){ext}\"; i=$((i+1)); done && {pack} && echo \"$f\"",
        quote(dir),
        quote(&base)
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zip_and_tar_keep_originals_and_unpack() {
        let root = std::env::temp_dir().join(format!("tror-archive-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join("site");
        std::fs::create_dir_all(folder.join("inner")).unwrap();
        std::fs::write(folder.join("index.php"), "<?php echo 1;").unwrap();
        std::fs::write(folder.join("inner").join("пример.txt"), "текст".repeat(100)).unwrap();
        let zip = create(std::slice::from_ref(&folder), Format::Zip).unwrap();
        assert_eq!(zip, root.join("site.zip"));
        let again = create(std::slice::from_ref(&folder), Format::Zip).unwrap();
        assert_eq!(again, root.join("site (2).zip"));
        let files = crate::plugins::unzip(&zip).unwrap();
        let names: Vec<&str> = files.iter().map(|(n, _)| n.as_str()).collect();
        assert!(
            names.contains(&"site/index.php") && names.contains(&"site/inner/пример.txt"),
            "{names:?}"
        );
        let text = files.iter().find(|(n, _)| n.ends_with(".txt")).unwrap();
        assert_eq!(String::from_utf8_lossy(&text.1), "текст".repeat(100));
        let tar = create(std::slice::from_ref(&folder), Format::TarGz).unwrap();
        assert!(folder.join("index.php").exists());
        if which("tar").is_some() {
            let out = root.join("out");
            std::fs::create_dir_all(&out).unwrap();
            let status = Command::new("tar")
                .arg("-xzf")
                .arg(&tar)
                .arg("-C")
                .arg(&out)
                .status()
                .unwrap();
            assert!(status.success());
            let text =
                std::fs::read_to_string(out.join("site").join("inner").join("пример.txt")).unwrap();
            assert_eq!(text, "текст".repeat(100));
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn remote_command_quotes_names() {
        let cmd = remote_command("/var/www", &["it's".into()], false);
        assert!(cmd.starts_with("cd '/var/www' && n='it'\\''s'"), "{cmd}");
        assert!(cmd.contains("tar -czf \"$f\" -- 'it'\\''s'"), "{cmd}");
    }
}
