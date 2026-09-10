use clipkeep_plugin_sdk::{Plugin, run_plugin};
use clipkeep_protocol::{ConfigurationItem, ResultAction, ResultKind, SearchResult};
use std::{
    collections::{BTreeMap, HashSet},
    io,
    process::{Command, ExitStatus},
};

const PLUGIN_ID: &str = "io.clipkeep.system-commands";

fn main() -> io::Result<()> {
    run_plugin(SystemCommandsPlugin::default())
}

#[derive(Default)]
struct SystemCommandsPlugin {
    hidden_items: HashSet<String>,
}

#[derive(Clone, Copy)]
struct SystemCommand {
    id: &'static str,
    title: &'static str,
    subtitle: &'static str,
    keywords: &'static str,
    destructive: bool,
}

const COMMANDS: &[SystemCommand] = &[
    SystemCommand {
        id: "lock",
        title: "Lock Screen",
        subtitle: "Require authentication to return",
        keywords: "lock screen 锁屏",
        destructive: false,
    },
    SystemCommand {
        id: "display-sleep",
        title: "Turn Display Off",
        subtitle: "Put displays to sleep immediately",
        keywords: "display sleep screen off 关闭显示器",
        destructive: false,
    },
    SystemCommand {
        id: "screen-saver",
        title: "Start Screen Saver",
        subtitle: "Start the macOS screen saver",
        keywords: "screen saver 屏幕保护",
        destructive: false,
    },
    SystemCommand {
        id: "sleep",
        title: "Sleep Mac",
        subtitle: "Put this Mac to sleep",
        keywords: "sleep mac 睡眠",
        destructive: false,
    },
    SystemCommand {
        id: "settings",
        title: "Open System Settings",
        subtitle: "Open the System Settings application",
        keywords: "settings preferences 设置",
        destructive: false,
    },
    SystemCommand {
        id: "force-quit",
        title: "Force Quit Applications",
        subtitle: "Show the Force Quit window",
        keywords: "force quit applications 强制退出",
        destructive: false,
    },
    SystemCommand {
        id: "log-out",
        title: "Log Out",
        subtitle: "Log out of the current macOS account",
        keywords: "log out logout 注销",
        destructive: true,
    },
    SystemCommand {
        id: "restart",
        title: "Restart Mac",
        subtitle: "Restart this Mac",
        keywords: "restart reboot 重启",
        destructive: true,
    },
    SystemCommand {
        id: "shut-down",
        title: "Shut Down Mac",
        subtitle: "Shut down this Mac",
        keywords: "shutdown power off 关机",
        destructive: true,
    },
];

impl Plugin for SystemCommandsPlugin {
    fn id(&self) -> &'static str {
        PLUGIN_ID
    }

    fn search(&mut self, query: &str, limit: usize) -> Result<Vec<SearchResult>, String> {
        let query = query.trim().to_lowercase();
        Ok(COMMANDS
            .iter()
            .filter(|command| !self.hidden_items.contains(command.id))
            .filter(|command| matches_query(command, &query))
            .take(limit)
            .enumerate()
            .map(|(index, command)| as_result(*command, index))
            .collect())
    }

    fn execute(&mut self, result_id: &str, action_id: &str) -> Result<Option<String>, String> {
        if action_id != "run" {
            return Err(format!("unknown system action: {action_id}"));
        }
        let id = result_id
            .strip_prefix("system-command:")
            .ok_or_else(|| "invalid system command result".to_owned())?;
        let status = match id {
            "lock" => apple_script("tell application \"System Events\" to keystroke \"q\" using {control down, command down}"),
            "display-sleep" => Command::new("/usr/bin/pmset").arg("displaysleepnow").status(),
            "screen-saver" => Command::new("/usr/bin/open").args(["-a", "ScreenSaverEngine"]).status(),
            "sleep" => apple_script("tell application \"System Events\" to sleep"),
            "settings" => Command::new("/usr/bin/open").arg("x-apple.systempreferences:").status(),
            "force-quit" => apple_script("tell application \"System Events\" to key code 53 using {option down, command down}"),
            "log-out" => apple_script("tell application \"System Events\" to log out"),
            "restart" => apple_script("tell application \"System Events\" to restart"),
            "shut-down" => apple_script("tell application \"System Events\" to shut down"),
            _ => return Err("unknown system command".into()),
        }.map_err(|error| error.to_string())?;
        if !status.success() {
            return Err(format!("system command failed with {status}"));
        }
        Ok(Some("System command sent".into()))
    }

    fn configure(&mut self, settings: &BTreeMap<String, String>) -> Result<bool, String> {
        let hidden_items = settings
            .get("hidden_items")
            .map(|value| serde_json::from_str(value))
            .transpose()
            .map_err(|error| format!("invalid hidden system command list: {error}"))?
            .unwrap_or_default();
        let changed = hidden_items != self.hidden_items;
        self.hidden_items = hidden_items;
        Ok(changed)
    }

    fn configuration_items(&self) -> Vec<ConfigurationItem> {
        COMMANDS
            .iter()
            .map(|command| ConfigurationItem {
                id: command.id.into(),
                title: command.title.into(),
                subtitle: command.subtitle.into(),
                metadata: BTreeMap::from([(
                    "symbol_name".into(),
                    command_symbol(command.id).into(),
                )]),
            })
            .collect()
    }
}

fn apple_script(script: &str) -> io::Result<ExitStatus> {
    Command::new("/usr/bin/osascript")
        .args(["-e", script])
        .status()
}

fn matches_query(command: &SystemCommand, query: &str) -> bool {
    query.is_empty()
        || query.split_whitespace().all(|word| {
            command.title.to_lowercase().contains(word)
                || command.keywords.to_lowercase().contains(word)
        })
}

fn as_result(command: SystemCommand, index: usize) -> SearchResult {
    SearchResult {
        plugin_id: PLUGIN_ID.into(),
        id: format!("system-command:{}", command.id),
        title: command.title.into(),
        subtitle: command.subtitle.into(),
        preview: command.subtitle.into(),
        kind: ResultKind::Command,
        score: 2_000.0 - index as f32,
        metadata: BTreeMap::from([
            ("command".into(), command.id.into()),
            ("symbol_name".into(), command_symbol(command.id).into()),
        ]),
        actions: vec![ResultAction {
            id: "run".into(),
            title: command.title.into(),
            destructive: command.destructive,
            default: true,
            closes_window: true,
            paste_to_previous: false,
            input_fields: Vec::new(),
        }],
    }
}

fn command_symbol(id: &str) -> &'static str {
    match id {
        "lock" => "lock",
        "display-sleep" => "display",
        "screen-saver" => "sparkles.rectangle.stack",
        "sleep" => "moon.zzz",
        "settings" => "gearshape",
        "force-quit" => "xmark.app",
        "log-out" => "rectangle.portrait.and.arrow.right",
        "restart" => "restart",
        "shut-down" => "power",
        _ => "terminal",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn searches_english_and_chinese_keywords() {
        assert!(matches_query(&COMMANDS[0], "lock"));
        assert!(matches_query(&COMMANDS[0], "锁屏"));
        assert!(!matches_query(&COMMANDS[0], "restart"));
    }

    #[test]
    fn configured_hidden_commands_are_excluded_from_search() {
        let mut plugin = SystemCommandsPlugin::default();
        let settings = BTreeMap::from([(
            "hidden_items".into(),
            serde_json::to_string(&["lock"]).unwrap(),
        )]);

        assert!(plugin.configure(&settings).unwrap());
        assert!(plugin.search("lock", 10).unwrap().is_empty());
        assert_eq!(plugin.configuration_items().len(), COMMANDS.len());
    }

    #[test]
    fn every_command_has_a_unique_id() {
        let mut ids: Vec<_> = COMMANDS.iter().map(|command| command.id).collect();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), COMMANDS.len());
    }
}
