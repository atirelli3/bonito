//! The command line: `bonito [--kuna PATH] <binary>`.

use std::ffi::OsString;
use std::path::PathBuf;

/// What `bonito --help` prints.
pub const USAGE: &str = "usage: bonito [--kuna PATH] [--] <binary>

  <binary>      the program to analyse; it is never run, only handed to kuna
  --kuna PATH   the kuna to use (default: $BONITO_KUNA, then kuna on PATH)
  -h, --help    print this help";

/// What the command line asks for.
#[derive(Debug, PartialEq)]
pub enum Command {
    /// Print the usage.
    Help,
    /// Open `binary`, with the kuna at `kuna` if one was named.
    Open {
        binary: PathBuf,
        kuna: Option<PathBuf>,
    },
}

/// Reads the arguments after the program name; the error says what is wrong with them.
pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Command, String> {
    let args: Vec<OsString> = args.into_iter().collect();
    let options = args.iter().take_while(|arg| arg.to_str() != Some("--"));
    if options
        .into_iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
    {
        return Ok(Command::Help);
    }
    let mut args = args.into_iter();
    let mut binary: Option<PathBuf> = None;
    let mut kuna: Option<PathBuf> = None;
    let mut options_over = false;
    while let Some(arg) = args.next() {
        if arg.is_empty() {
            return Err("empty argument".to_string());
        }
        match arg.to_str() {
            Some("--") if !options_over => options_over = true,
            Some("--kuna") if !options_over => {
                let path = args.next().ok_or("--kuna needs a PATH")?;
                set_kuna(&mut kuna, path)?;
            }
            Some(flag) if !options_over && flag.starts_with("--kuna=") => {
                set_kuna(&mut kuna, OsString::from(&flag["--kuna=".len()..]))?;
            }
            Some(flag) if !options_over && flag.starts_with('-') => {
                return Err(format!("unknown option {flag:?}"));
            }
            _ if binary.is_some() => return Err(format!("unexpected argument {arg:?}")),
            _ => binary = Some(PathBuf::from(arg)),
        }
    }
    let binary = binary.ok_or("missing <binary>")?;
    Ok(Command::Open { binary, kuna })
}

/// Records the kuna named on the command line; naming it twice or naming nothing is an error.
fn set_kuna(kuna: &mut Option<PathBuf>, path: OsString) -> Result<(), String> {
    if path.is_empty() {
        return Err("--kuna needs a PATH".to_string());
    }
    match kuna.replace(PathBuf::from(path)) {
        Some(_) => Err("--kuna was given twice".to_string()),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<Command, String> {
        parse(args.iter().map(OsString::from))
    }

    fn open(binary: &str, kuna: Option<&str>) -> Command {
        Command::Open {
            binary: PathBuf::from(binary),
            kuna: kuna.map(PathBuf::from),
        }
    }

    #[test]
    fn a_binary_alone_is_opened() {
        assert_eq!(parsed(&["hello.elf"]), Ok(open("hello.elf", None)));
    }

    #[test]
    fn kuna_can_come_before_or_after_the_binary() {
        let expected = Ok(open("a.elf", Some("/opt/kuna")));
        assert_eq!(parsed(&["--kuna", "/opt/kuna", "a.elf"]), expected);
        assert_eq!(parsed(&["a.elf", "--kuna", "/opt/kuna"]), expected);
    }

    #[test]
    fn help_wins_wherever_it_is_even_over_an_error() {
        assert_eq!(parsed(&["-h"]), Ok(Command::Help));
        assert_eq!(parsed(&["a.elf", "--help"]), Ok(Command::Help));
        assert_eq!(parsed(&["--bad", "--help"]), Ok(Command::Help));
        assert_eq!(
            parsed(&["--kuna", "a", "--kuna", "b", "-h"]),
            Ok(Command::Help)
        );
    }

    #[test]
    fn kuna_can_be_given_with_an_equals_sign() {
        assert_eq!(
            parsed(&["--kuna=/opt/kuna", "a.elf"]),
            Ok(open("a.elf", Some("/opt/kuna")))
        );
        assert!(parsed(&["--kuna=", "a.elf"]).is_err());
    }

    #[test]
    fn an_empty_argument_is_an_error() {
        assert_eq!(parsed(&[""]), Err("empty argument".to_string()));
        assert!(parsed(&["--kuna", "", "a.elf"]).is_err());
    }

    #[test]
    fn a_binary_that_looks_like_a_flag_follows_the_double_dash() {
        assert_eq!(parsed(&["--", "-x"]), Ok(open("-x", None)));
        assert_eq!(parsed(&["--", "--help"]), Ok(open("--help", None)));
    }

    #[test]
    fn a_missing_binary_is_an_error() {
        assert_eq!(parsed(&[]), Err("missing <binary>".to_string()));
        let error = parsed(&["--kuna", "k"]).expect_err("no binary");
        assert!(error.contains("missing"), "{error}");
    }

    #[test]
    fn kuna_without_a_path_is_an_error() {
        let error = parsed(&["a.elf", "--kuna"]).expect_err("no path");
        assert!(error.contains("--kuna needs"), "{error}");
    }

    #[test]
    fn kuna_twice_is_an_error() {
        let error = parsed(&["--kuna", "a", "--kuna", "b", "x"]).expect_err("twice");
        assert!(error.contains("twice"), "{error}");
    }

    #[test]
    fn an_unknown_option_is_an_error_and_is_printed_escaped() {
        let error = parsed(&["--evil\u{1b}[2J", "a.elf"]).expect_err("unknown");
        assert_eq!(error, r#"unknown option "--evil\u{1b}[2J""#);
    }

    #[test]
    fn a_second_binary_is_an_error() {
        let error = parsed(&["a.elf", "b.elf"]).expect_err("two");
        assert!(error.contains("unexpected argument"), "{error}");
    }
}
