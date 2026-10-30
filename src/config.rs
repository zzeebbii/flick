//! `~/.config/flick/config.toml` (Q15) and Themes. The background process reads the Source
//! settings; the shell gets the rest as zsh assignments from `flick config zsh`.
use serde::Deserialize;
use std::path::{Path, PathBuf};

pub const EXAMPLE: &str = r#"# flick config. Every key is optional; these are the defaults.

trigger = "typing"   # "typing": the Menu opens as you type. "tab": only when you press Tab.
rows = 8             # Menu rows (1-20)
align = "left"       # "left": the Menu starts at the left edge. "cursor": under the word being typed.
theme = "default"    # "default", "nerd" (needs a Nerd Font), or a file in themes/<name>.toml
preset = "all"       # "all", "history-only" or "specs-only"

# Turn single Sources on or off; overrides the preset.
[sources]
# history = true     # History lines and History-based ranking
# specs = true       # subcommands, options and values from Specs, and command names
# generators = true  # values from running commands (branches, containers, ...)
# files = true       # file and folder names
# native = true      # Tab hands over to zsh's own completion when flick has nothing

[history]
# Never recorded (case-insensitive substrings). Also applied when History is loaded.
ignore = ["password", "token=", "secret", "authorization:", "bearer ", "ghp_", "github_pat_", "aiza"]

[generators]
disable = []         # CLIs whose Generators never run, e.g. ["aws", "gcloud"] to avoid network calls

# bindkey notation: ^I = Tab, ^[ = Esc, ^[[A/^[[B/^[[C = Up/Down/Right.
[keys]
accept = "^I"
accept-ghost = "^[[C"
up = "^[[A"
down = "^[[B"
close = "^["
"#;

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub trigger: String,
    pub rows: u8,
    pub align: String,
    pub theme: String,
    pub preset: String,
    pub sources: SourceSwitches,
    pub history: HistoryConfig,
    pub generators: GeneratorConfig,
    pub keys: Keys,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct SourceSwitches {
    pub history: Option<bool>,
    pub specs: Option<bool>,
    pub generators: Option<bool>,
    pub files: Option<bool>,
    pub native: Option<bool>,
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields)]
pub struct HistoryConfig {
    pub ignore: Vec<String>,
}

#[derive(Deserialize, Debug, Default)]
#[serde(default, deny_unknown_fields)]
pub struct GeneratorConfig {
    pub disable: Vec<String>,
}

#[derive(Deserialize, Debug)]
#[serde(default, deny_unknown_fields, rename_all = "kebab-case")]
pub struct Keys {
    pub accept: String,
    pub accept_ghost: String,
    pub up: String,
    pub down: String,
    pub close: String,
}

/// Which Sources are on, after applying the preset and then `[sources]`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Sources {
    pub history: bool,
    pub specs: bool,
    pub generators: bool,
    pub files: bool,
    pub native: bool,
}

#[derive(Deserialize, Debug, Clone)]
#[serde(default, deny_unknown_fields)]
pub struct Theme {
    /// zsh highlight specs (`fg=8`, `bold`, `standout`, ...), as in `region_highlight`.
    pub ghost: String,
    pub name: String,
    pub description: String,
    pub selected: String,
    pub history: String,
    pub icons: Icons,
}

#[derive(Deserialize, Debug, Clone, Default)]
#[serde(default, deny_unknown_fields)]
pub struct Icons {
    pub history: String,
    pub command: String,
    pub option: String,
    pub file: String,
    pub folder: String,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            trigger: "typing".into(),
            rows: 8,
            align: "left".into(),
            theme: "default".into(),
            preset: "all".into(),
            sources: SourceSwitches::default(),
            history: HistoryConfig::default(),
            generators: GeneratorConfig::default(),
            keys: Keys::default(),
        }
    }
}

impl Default for HistoryConfig {
    fn default() -> Self {
        // Also common credential shapes: GitHub tokens (ghp_, github_pat_), Google API keys
        // (AIza...), HTTP auth headers.
        let patterns = [
            "password",
            "token=",
            "secret",
            "authorization:",
            "bearer ",
            "ghp_",
            "github_pat_",
            "aiza",
        ];
        HistoryConfig {
            ignore: patterns.map(String::from).to_vec(),
        }
    }
}

impl Default for Keys {
    fn default() -> Self {
        Keys {
            accept: "^I".into(),
            accept_ghost: "^[[C".into(),
            up: "^[[A".into(),
            down: "^[[B".into(),
            close: "^[".into(),
        }
    }
}

impl Default for Theme {
    /// Uses the terminal's own palette, so it follows the terminal's light or dark scheme.
    fn default() -> Self {
        Theme {
            ghost: "fg=8".into(),
            name: "fg=6".into(),
            description: "fg=8".into(),
            selected: "standout".into(),
            history: "fg=5".into(),
            icons: Icons::default(),
        }
    }
}

impl Theme {
    pub fn builtin(name: &str) -> Option<Theme> {
        match name {
            "default" => Some(Theme::default()),
            // Nerd Font glyphs: clock, terminal, flag, file, folder.
            "nerd" => Some(Theme {
                name: "fg=4,bold".into(),
                selected: "fg=0,bg=4".into(),
                icons: Icons {
                    history: "\u{f017}".into(),
                    command: "\u{f489}".into(),
                    option: "\u{f024}".into(),
                    file: "\u{f15b}".into(),
                    folder: "\u{f07b}".into(),
                },
                ..Theme::default()
            }),
            _ => None,
        }
    }
}

pub fn config_dir() -> PathBuf {
    if let Some(p) = std::env::var_os("FLICK_CONFIG") {
        return PathBuf::from(p)
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
    }
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        });
    base.join("flick")
}

/// `$FLICK_CONFIG`, else `$XDG_CONFIG_HOME/flick/config.toml`, else `~/.config/flick/config.toml`.
pub fn config_path() -> PathBuf {
    std::env::var_os("FLICK_CONFIG").map_or_else(|| config_dir().join("config.toml"), PathBuf::from)
}

impl Config {
    /// A missing file means defaults; a broken one is an error (and the caller uses defaults).
    pub fn load(path: &Path) -> Result<Config, String> {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Config::default()),
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        let c: Config =
            toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e.message()))?;
        c.validate()
            .map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(c)
    }

    fn validate(&self) -> Result<(), String> {
        if !matches!(self.trigger.as_str(), "typing" | "tab") {
            return Err(format!(
                "trigger must be \"typing\" or \"tab\", not {:?}",
                self.trigger
            ));
        }
        if !matches!(self.align.as_str(), "left" | "cursor") {
            return Err(format!(
                "align must be \"left\" or \"cursor\", not {:?}",
                self.align
            ));
        }
        if !(1..=20).contains(&self.rows) {
            return Err(format!("rows must be 1-20, not {}", self.rows));
        }
        if !matches!(self.preset.as_str(), "all" | "history-only" | "specs-only") {
            return Err(format!(
                "preset must be \"all\", \"history-only\" or \"specs-only\", not {:?}",
                self.preset
            ));
        }
        Ok(())
    }

    pub fn sources(&self) -> Sources {
        let (history, others) = match self.preset.as_str() {
            "history-only" => (true, false),
            "specs-only" => (false, true),
            _ => (true, true),
        };
        let s = &self.sources;
        Sources {
            history: s.history.unwrap_or(history),
            specs: s.specs.unwrap_or(others),
            generators: s.generators.unwrap_or(others),
            files: s.files.unwrap_or(others),
            native: s.native.unwrap_or(true),
        }
    }

    /// Looks up `theme` among the built-ins, then `themes/<name>.toml` next to the config.
    pub fn theme(&self) -> Result<Theme, String> {
        if let Some(t) = Theme::builtin(&self.theme) {
            return Ok(t);
        }
        let path = config_dir()
            .join("themes")
            .join(format!("{}.toml", self.theme));
        let text = std::fs::read_to_string(&path)
            .map_err(|e| format!("theme {:?}: {}: {e}", self.theme, path.display()))?;
        toml::from_str(&text).map_err(|e| format!("{}: {}", path.display(), e.message()))
    }
}

/// The shell's settings as zsh assignments, sourced by flick.zsh. `error` is shown once in
/// the shell (startup must stay silent).
pub fn to_zsh(c: &Config, t: &Theme, error: Option<&str>) -> String {
    let q = |s: &str| format!("'{}'", s.replace('\'', "'\\''"));
    let s = c.sources();
    let mut out = String::from("# Generated by `flick config zsh`; edit config.toml instead.\n");
    let mut set = |k: &str, v: String| out.push_str(&format!("typeset -g {k}={v}\n"));
    set("FLICK_TRIGGER", q(&c.trigger));
    set("FLICK_ROWS", c.rows.to_string());
    set("FLICK_ALIGN", q(&c.align));
    set("FLICK_NATIVE", u8::from(s.native).to_string());
    for (k, v) in [
        ("FLICK_STYLE_GHOST", &t.ghost),
        ("FLICK_STYLE_NAME", &t.name),
        ("FLICK_STYLE_DESC", &t.description),
        ("FLICK_STYLE_SELECTED", &t.selected),
        ("FLICK_STYLE_HISTORY", &t.history),
        ("FLICK_ICON_HISTORY", &t.icons.history),
        ("FLICK_ICON_COMMAND", &t.icons.command),
        ("FLICK_ICON_OPTION", &t.icons.option),
        ("FLICK_ICON_FILE", &t.icons.file),
        ("FLICK_ICON_FOLDER", &t.icons.folder),
        ("FLICK_KEY_ACCEPT", &c.keys.accept),
        ("FLICK_KEY_ACCEPT_GHOST", &c.keys.accept_ghost),
        ("FLICK_KEY_UP", &c.keys.up),
        ("FLICK_KEY_DOWN", &c.keys.down),
        ("FLICK_KEY_CLOSE", &c.keys.close),
    ] {
        set(k, q(v));
    }
    set("FLICK_CONFIG_ERROR", q(error.unwrap_or_default()));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(s: &str) -> Result<Config, String> {
        let c: Config = toml::from_str(s).map_err(|e| e.message().to_string())?;
        c.validate()?;
        Ok(c)
    }

    #[test]
    fn example_parses_to_defaults() {
        let c = parse(EXAMPLE).unwrap();
        assert_eq!(
            (c.trigger.as_str(), c.rows, c.theme.as_str()),
            ("typing", 8, "default")
        );
        assert_eq!(c.history.ignore, HistoryConfig::default().ignore);
    }

    #[test]
    fn presets_then_switches() {
        let s = parse(r#"preset = "history-only""#).unwrap().sources();
        assert!(s.history && !s.specs && !s.generators && !s.files && s.native);
        let s = parse("preset = \"history-only\"\n[sources]\nfiles = true")
            .unwrap()
            .sources();
        assert!(s.files && !s.specs);
        assert!(!parse(r#"preset = "specs-only""#).unwrap().sources().history);
    }

    #[test]
    fn bad_values_are_errors() {
        assert!(parse(r#"trigger = "sometimes""#).is_err());
        assert!(parse("rows = 0").is_err());
        assert!(parse(r#"align = "center""#).is_err());
        assert_eq!(parse(r#"align = "cursor""#).unwrap().align, "cursor");
        assert!(parse("colour = 1").is_err()); // unknown key: a typo shouldn't be silently ignored
    }

    #[test]
    fn zsh_output_is_quoted() {
        let mut c = Config::default();
        c.keys.accept = "it's".into();
        let z = to_zsh(&c, &Theme::default(), Some("bad 'x'"));
        assert!(z.contains("typeset -g FLICK_KEY_ACCEPT='it'\\''s'"));
        assert!(z.contains("typeset -g FLICK_CONFIG_ERROR='bad '\\''x'\\'''"));
        assert!(z.contains("typeset -g FLICK_ROWS=8"));
        assert!(z.contains("typeset -g FLICK_ALIGN='left'"));
    }

    #[test]
    fn nerd_theme_has_icons() {
        assert!(!Theme::builtin("nerd").unwrap().icons.folder.is_empty());
        assert!(Theme::builtin("nope").is_none());
    }
}
