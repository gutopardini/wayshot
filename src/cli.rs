use std::env::ArgsOs;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use crate::capture::CaptureMode;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchOptions {
    pub image: Option<PathBuf>,
    pub capture: bool,
    pub mode: CaptureMode,
    pub save_dir: Option<PathBuf>,
    pub clipboard: bool,
    pub raw: bool,
    pub format: Option<String>,
    pub delay: Option<Duration>,
}

impl LaunchOptions {
    pub fn headless(&self) -> bool {
        self.save_dir.is_some() || self.clipboard || self.raw
    }

    pub fn wants_capture(&self) -> bool {
        self.capture || self.headless() || self.mode != CaptureMode::Auto || self.delay.is_some()
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Run(LaunchOptions),
}

pub fn parse(args: ArgsOs, capture_from_env: bool) -> Result<CliCommand, String> {
    parse_os(args.skip(1), capture_from_env)
}

pub fn parse_os(
    args: impl IntoIterator<Item = OsString>,
    capture_from_env: bool,
) -> Result<CliCommand, String> {
    let mut options = LaunchOptions {
        image: None,
        capture: capture_from_env,
        mode: CaptureMode::Auto,
        save_dir: None,
        clipboard: false,
        raw: false,
        format: None,
        delay: None,
    };
    let mut positional = Vec::new();
    let mut args = args.into_iter().peekable();
    let mut end_of_flags = false;

    while let Some(arg) = args.next() {
        if end_of_flags {
            positional.push(arg);
            continue;
        }
        let Some(text) = arg.to_str() else {
            positional.push(arg);
            continue;
        };
        if text == "--" {
            end_of_flags = true;
            continue;
        }
        if text.is_empty() {
            continue;
        }
        if !text.starts_with('-') {
            positional.push(arg);
            continue;
        }

        let (flag, inline) = split_flag(text);
        match flag {
            "--help" | "-h" => return Ok(CliCommand::Help),
            "--capture" | "-c" => options.capture = true,
            "--fullscreen" => set_mode(&mut options, CaptureMode::Fullscreen)?,
            "--region" => set_mode(&mut options, CaptureMode::Region)?,
            "--clipboard" => options.clipboard = true,
            "--raw" => options.raw = true,
            "--path" => {
                options.save_dir = Some(value("--path", inline, &mut args)?);
            }
            "--format" => {
                let format = value_text("--format", inline, &mut args)?;
                options.format = Some(normalize_format(&format)?);
            }
            "--delay" => {
                let delay = value_text("--delay", inline, &mut args)?;
                options.delay = Some(parse_delay(&delay)?);
            }
            other => return Err(format!("unknown option '{other}'")),
        }
    }

    if positional.len() > 1 {
        return Err("only one image path can be passed".to_string());
    }
    if let Some(path) = positional.pop() {
        if options.headless() || options.mode != CaptureMode::Auto || options.delay.is_some() {
            return Err(
                "an image path cannot be combined with capture options; pass either a file or a capture command"
                    .to_string(),
            );
        }
        options.image = Some(PathBuf::from(path));
        options.capture = false;
    }

    Ok(CliCommand::Run(options))
}

pub const HELP: &str = "\
WayShot captures a screenshot through the desktop portal and opens it for annotation.

Usage:
  wayshot [IMAGE]
  wayshot [OPTIONS]

Capture:
  -c, --capture           Capture, then open the editor
      --fullscreen        Capture the whole screen and skip region selection
      --region            Ask for an interactive region or window selection
      --delay SECONDS     Wait before capturing

Output, instead of opening the editor:
      --path DIR          Save to DIR with a generated filename
      --clipboard         Copy the image and exit
      --raw               Write the image to stdout
      --format FMT        Output format for this run: png, jpeg, webp, jxl, avif, tiff

--path and --clipboard can be used together. Without an output option, WayShot
opens the editor after capture. Press Esc in the screenshot UI to cancel without
opening WayShot.

Environment:
  WAYSHOT_CAPTURE=1       Same as --capture
";

fn set_mode(options: &mut LaunchOptions, mode: CaptureMode) -> Result<(), String> {
    if options.mode != CaptureMode::Auto && options.mode != mode {
        return Err("--fullscreen and --region cannot be used together".to_string());
    }
    options.mode = mode;
    Ok(())
}

fn split_flag(text: &str) -> (&str, Option<&str>) {
    let Some((flag, value)) = text.split_once('=') else {
        return (text, None);
    };
    if flag == "--path" || flag == "--format" || flag == "--delay" {
        (flag, Some(value))
    } else {
        (text, None)
    }
}

fn value(
    flag: &str,
    inline: Option<&str>,
    args: &mut std::iter::Peekable<impl Iterator<Item = OsString>>,
) -> Result<PathBuf, String> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(format!("{flag} needs a value"));
        }
        return Ok(PathBuf::from(value));
    }
    args.next()
        .filter(|value| value.to_str().is_none_or(|text| !text.starts_with('-')))
        .map(PathBuf::from)
        .ok_or_else(|| format!("{flag} needs a value"))
}

fn value_text(
    flag: &str,
    inline: Option<&str>,
    args: &mut std::iter::Peekable<impl Iterator<Item = OsString>>,
) -> Result<String, String> {
    if let Some(value) = inline {
        if value.is_empty() {
            return Err(format!("{flag} needs a value"));
        }
        return Ok(value.to_string());
    }
    let value = args.next().ok_or_else(|| format!("{flag} needs a value"))?;
    let Some(text) = value.to_str() else {
        return Err(format!("{flag} needs a text value"));
    };
    if text.starts_with('-') {
        return Err(format!("{flag} needs a value"));
    }
    Ok(text.to_string())
}

pub fn normalize_format(format: &str) -> Result<String, String> {
    match format.to_ascii_lowercase().as_str() {
        "png" => Ok("png".to_string()),
        "jpg" | "jpeg" => Ok("jpeg".to_string()),
        "webp" => Ok("webp".to_string()),
        "jxl" => Ok("jxl".to_string()),
        "avif" => Ok("avif".to_string()),
        "tif" | "tiff" => Ok("tiff".to_string()),
        other => Err(format!(
            "unsupported format '{other}'. Use png, jpeg, webp, jxl, avif, or tiff"
        )),
    }
}

fn parse_delay(value: &str) -> Result<Duration, String> {
    let seconds: f64 = value
        .parse()
        .map_err(|_| format!("--delay needs a number of seconds, got '{value}'"))?;
    if !seconds.is_finite() || seconds < 0.0 {
        return Err("--delay needs a positive number of seconds".to_string());
    }
    if seconds > 300.0 {
        return Err("--delay cannot be longer than 300 seconds".to_string());
    }
    Ok(Duration::from_secs_f64(seconds))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_args(args: &[&str]) -> Result<CliCommand, String> {
        parse_os(args.iter().map(OsString::from), false)
    }

    fn run(args: &[&str]) -> LaunchOptions {
        match parse_args(args).unwrap() {
            CliCommand::Run(options) => options,
            CliCommand::Help => panic!("expected run options"),
        }
    }

    #[test]
    fn defaults_to_opening_the_editor() {
        let options = run(&[]);
        assert!(!options.wants_capture());
        assert!(!options.headless());
        assert_eq!(options.mode, CaptureMode::Auto);
    }

    #[test]
    fn capture_flags_select_the_outcome_before_the_editor() {
        let options = run(&[
            "--fullscreen",
            "--path",
            "/tmp/shots",
            "--clipboard",
            "--format",
            "jpg",
            "--delay",
            "1.5",
        ]);
        assert!(options.headless());
        assert!(options.wants_capture());
        assert_eq!(options.mode, CaptureMode::Fullscreen);
        assert_eq!(options.save_dir.unwrap(), PathBuf::from("/tmp/shots"));
        assert!(options.clipboard);
        assert_eq!(options.format.as_deref(), Some("jpeg"));
        assert_eq!(options.delay, Some(Duration::from_secs_f64(1.5)));
    }

    #[test]
    fn environment_capture_still_opens_the_editor() {
        let options = parse_os(std::iter::empty(), true).unwrap();
        let CliCommand::Run(options) = options else {
            panic!("expected run");
        };
        assert!(options.capture);
        assert!(options.wants_capture());
        assert!(!options.headless());
    }

    #[test]
    fn image_argument_does_not_capture() {
        let options = run(&["shot.png"]);
        assert_eq!(
            options.image.as_deref(),
            Some(std::path::Path::new("shot.png"))
        );
        assert!(!options.wants_capture());
    }

    #[test]
    fn rejects_conflicting_capture_shapes() {
        assert!(parse_args(&["--fullscreen", "--region"]).is_err());
        assert!(parse_args(&["shot.png", "--clipboard"]).is_err());
        assert!(parse_args(&["--path"]).is_err());
        assert!(parse_args(&["--format", "gif"]).is_err());
        assert!(parse_args(&["--nope"]).is_err());
    }
}
