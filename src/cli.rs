//! Command line and environment of the server binary.
use std::path::PathBuf;

pub const USAGE: &str = "usage: blender-live <BASE_CAMP>

Environment:
  PORT          HTTP port (default 5014)
  LOG_LEVEL     off, error, warn, info, debug or trace (default info)
  BLENDER_BIN   Blender executable (default: blender on PATH)
  BLENDER_GPUS  nvidia-smi indices Cycles may use, e.g. 0 or 0,1, or all (default 0,1)";

#[derive(Debug, PartialEq, Eq)]
pub enum Cli {
    Serve(PathBuf),
    Version,
    Help,
}

pub fn parse(args: &[String]) -> Result<Cli, String> {
    match args {
        [flag] if flag == "--version" || flag == "-V" => Ok(Cli::Version),
        [flag] if flag == "--help" || flag == "-h" => Ok(Cli::Help),
        [camp] if !camp.starts_with('-') => Ok(Cli::Serve(PathBuf::from(camp))),
        [] => Err("missing BASE_CAMP".into()),
        _ => Err(format!("unexpected arguments: {}", args.join(" "))),
    }
}

/// `CUDA_VISIBLE_DEVICES` for Blender; `None` leaves every GPU visible.
pub fn gpus(value: Option<&str>) -> Result<Option<String>, String> {
    let value = value
        .map(str::trim)
        .filter(|v| !v.is_empty())
        .unwrap_or("0,1");
    if value == "all" {
        return Ok(None);
    }
    let valid = value
        .split(',')
        .all(|index| !index.is_empty() && index.bytes().all(|b| b.is_ascii_digit()));
    if valid {
        Ok(Some(value.to_owned()))
    } else {
        Err(format!(
            "BLENDER_GPUS must be indices like 0 or 0,1, or all; got {value:?}"
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn one_base_camp_is_served() {
        assert_eq!(
            parse(&args(&["/srv/camp"])),
            Ok(Cli::Serve(PathBuf::from("/srv/camp")))
        );
    }

    #[test]
    fn version_and_help_flags() {
        assert_eq!(parse(&args(&["--version"])), Ok(Cli::Version));
        assert_eq!(parse(&args(&["-V"])), Ok(Cli::Version));
        assert_eq!(parse(&args(&["--help"])), Ok(Cli::Help));
        assert_eq!(parse(&args(&["-h"])), Ok(Cli::Help));
    }

    #[test]
    fn a_missing_or_extra_argument_is_an_error() {
        for list in [&[][..], &["a", "b"], &["--port", "1"]] {
            assert!(parse(&args(list)).is_err(), "{list:?}");
        }
    }

    #[test]
    fn gpus_default_to_both_cards() {
        assert_eq!(gpus(None), Ok(Some("0,1".into())));
        assert_eq!(gpus(Some("")), Ok(Some("0,1".into())));
    }

    #[test]
    fn gpus_take_index_lists_or_all() {
        assert_eq!(gpus(Some("0,1")), Ok(Some("0,1".into())));
        assert_eq!(gpus(Some(" 1 ")), Ok(Some("1".into())));
        assert_eq!(gpus(Some("all")), Ok(None));
        for bad in ["a", "0,", ",1", "0;1", "-1", "0 1"] {
            assert!(gpus(Some(bad)).is_err(), "accepted {bad:?}");
        }
    }
}
