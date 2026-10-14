use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use gdk_pixbuf::Pixbuf;
use gtk::gio;
use gtk::gio::prelude::FileExt;
use gtk::glib;

use crate::capture::CaptureFailure;
use crate::cli::LaunchOptions;
use crate::jxl;

struct ImageFormat {
    id: &'static str,
    extension: &'static str,
    pixbuf_type: &'static str,
}

fn image_format(id: &str) -> ImageFormat {
    match id {
        "jpeg" => ImageFormat {
            id: "jpeg",
            extension: "jpg",
            pixbuf_type: "jpeg",
        },
        "webp" => ImageFormat {
            id: "webp",
            extension: "webp",
            pixbuf_type: "webp",
        },
        "jxl" => ImageFormat {
            id: "jxl",
            extension: "jxl",
            pixbuf_type: "jxl",
        },
        "avif" => ImageFormat {
            id: "avif",
            extension: "avif",
            pixbuf_type: "avif",
        },
        "tiff" => ImageFormat {
            id: "tiff",
            extension: "tiff",
            pixbuf_type: "tiff",
        },
        _ => ImageFormat {
            id: "png",
            extension: "png",
            pixbuf_type: "png",
        },
    }
}

pub fn copy_png_with_wl_copy(pixbuf: &Pixbuf) -> Result<(), String> {
    let png = pixbuf
        .save_to_bufferv("png", &[])
        .map_err(|err| err.to_string())?;
    let mut child = Command::new("wl-copy")
        .args(["--type", "image/png"])
        .stdin(Stdio::piped())
        .spawn()
        .map_err(|err| err.to_string())?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| "Failed to open wl-copy stdin.".to_string())?;
    stdin.write_all(&png).map_err(|err| err.to_string())?;
    drop(stdin);
    Ok(())
}

pub fn deliver(options: &LaunchOptions, result: Result<String, CaptureFailure>) -> i32 {
    let uri = match result {
        Ok(uri) => uri,
        Err(error) if error.is_user_abort() => {
            eprintln!("Capture canceled.");
            return 1;
        }
        Err(error) => {
            eprintln!("Capture failed: {}", error.message);
            return 1;
        }
    };

    if let Err(error) = gtk::init() {
        eprintln!("Failed to initialize GTK: {error}");
        return 1;
    }

    match write_outputs(options, &uri) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn write_outputs(options: &LaunchOptions, uri: &str) -> Result<(), String> {
    let pixbuf = load_capture(uri)?;
    let format = image_format(options.format.as_deref().unwrap_or("png"));

    if options.clipboard {
        copy_png_with_wl_copy(&pixbuf)?;
    }
    if let Some(dir) = &options.save_dir {
        let path = output_path(dir, &timestamp(), format.extension)?;
        save_pixbuf(&pixbuf, &path, &format)?;
        if !options.raw {
            println!("{}", path.display());
        }
    }
    if options.raw {
        let bytes = encode(&pixbuf, &format)?;
        let mut stdout = io::stdout().lock();
        stdout.write_all(&bytes).map_err(|err| err.to_string())?;
        stdout.flush().map_err(|err| err.to_string())?;
    }
    Ok(())
}

fn load_capture(uri: &str) -> Result<Pixbuf, String> {
    let path = gio::File::for_uri(uri)
        .path()
        .ok_or_else(|| format!("Failed to resolve capture path: {uri}"))?;
    Pixbuf::from_file(&path).map_err(|err| format!("Failed to load capture: {err}"))
}

fn save_pixbuf(pixbuf: &Pixbuf, path: &Path, format: &ImageFormat) -> Result<(), String> {
    if format.id == "jxl" {
        return jxl::save(pixbuf, path);
    }
    pixbuf
        .savev(path, format.pixbuf_type, &[])
        .map_err(|err| err.to_string())
}

fn encode(pixbuf: &Pixbuf, format: &ImageFormat) -> Result<Vec<u8>, String> {
    if format.id == "jxl" {
        let path = std::env::temp_dir().join(format!(
            "wayshot-{}-{}.jxl",
            std::process::id(),
            timestamp()
        ));
        jxl::save(pixbuf, &path)?;
        let bytes = std::fs::read(&path).map_err(|err| err.to_string());
        let _ = std::fs::remove_file(&path);
        return bytes;
    }
    pixbuf
        .save_to_bufferv(format.pixbuf_type, &[])
        .map_err(|err| err.to_string())
}

fn timestamp() -> String {
    glib::DateTime::now_local()
        .ok()
        .and_then(|time| time.format("%Y%m%d-%H%M%S").ok())
        .map(|text| text.to_string())
        .unwrap_or_else(|| "capture".to_string())
}

pub fn output_path(dir: &Path, stamp: &str, extension: &str) -> Result<PathBuf, String> {
    if !dir.is_dir() {
        return Err(format!(
            "--path needs an existing directory, got {}",
            dir.display()
        ));
    }
    let candidate = dir.join(format!("wayshot-{stamp}.{extension}"));
    if !candidate.exists() {
        return Ok(candidate);
    }
    for index in 1..10_000 {
        let candidate = dir.join(format!("wayshot-{stamp}-{index}.{extension}"));
        if !candidate.exists() {
            return Ok(candidate);
        }
    }
    Err(format!(
        "could not find a free filename in {}",
        dir.display()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generated_name_avoids_collisions() {
        let dir = std::env::temp_dir().join(format!("wayshot-path-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let first = output_path(&dir, "20261013-120000", "png").unwrap();
        std::fs::write(&first, b"x").unwrap();
        let second = output_path(&dir, "20261013-120000", "png").unwrap();
        assert_ne!(first, second);
        assert!(second.ends_with("wayshot-20261013-120000-1.png"));
        let missing = output_path(Path::new("/no/such/wayshot-dir"), "stamp", "png");
        assert!(missing.is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
