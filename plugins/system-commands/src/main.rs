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
        id: "mute",
        title: "Toggle Mute",
        subtitle: "Mute or unmute system audio",
        keywords: "mute unmute audio sound volume 静音 取消静音 音量",
        destructive: false,
    },
    SystemCommand {
        id: "volume-up",
        title: "Volume Up",
        subtitle: "Increase the system audio volume",
        keywords: "volume up increase audio sound 调高 增加 音量",
        destructive: false,
    },
    SystemCommand {
        id: "volume-down",
        title: "Volume Down",
        subtitle: "Decrease the system audio volume",
        keywords: "volume down decrease audio sound 调低 减少 音量",
        destructive: false,
    },
    SystemCommand {
        id: "play-pause",
        title: "Play/Pause",
        subtitle: "Play or pause the current media session",
        keywords: "play pause media music 播放 暂停 音乐",
        destructive: false,
    },
    SystemCommand {
        id: "previous-track",
        title: "Previous Track",
        subtitle: "Return to the previous media track",
        keywords: "previous track song media music back 上一首 前一首 音乐",
        destructive: false,
    },
    SystemCommand {
        id: "next-track",
        title: "Next Track",
        subtitle: "Skip to the next media track",
        keywords: "next track song media music skip 下一首 音乐",
        destructive: false,
    },
    SystemCommand {
        id: "eject-all",
        title: "Eject All Mounted Disks",
        subtitle: "Eject every mounted disk that can be removed safely",
        keywords: "eject all mounted disk drive volume 弹出 所有 磁盘 移动硬盘 U盘",
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
        let message = match id {
            "lock" => run_status(
                apple_script(
                    "tell application \"System Events\" to keystroke \"q\" using {control down, command down}",
                ),
                "lock screen",
            )?,
            "display-sleep" => run_status(
                Command::new("/usr/bin/pmset")
                    .arg("displaysleepnow")
                    .status(),
                "turn display off",
            )?,
            "screen-saver" => run_status(
                Command::new("/usr/bin/open")
                    .args(["-a", "ScreenSaverEngine"])
                    .status(),
                "start screen saver",
            )?,
            "mute" | "volume-up" | "volume-down" | "play-pause" | "previous-track"
            | "next-track" => {
                return Err("media and volume commands must be handled by Nexus".into());
            }
            "eject-all" => run_status(
                apple_script(
                    "tell application \"Finder\" to eject every disk whose ejectable is true",
                ),
                "eject mounted disks",
            )?,
            "sleep" => run_status(
                apple_script("tell application \"System Events\" to sleep"),
                "sleep Mac",
            )?,
            "settings" => run_status(
                Command::new("/usr/bin/open")
                    .arg("x-apple.systempreferences:")
                    .status(),
                "open System Settings",
            )?,
            "force-quit" => run_status(
                apple_script(
                    "tell application \"System Events\" to key code 53 using {option down, command down}",
                ),
                "show Force Quit",
            )?,
            "log-out" => run_status(
                apple_script("tell application \"System Events\" to log out"),
                "log out",
            )?,
            "restart" => run_status(
                apple_script("tell application \"System Events\" to restart"),
                "restart Mac",
            )?,
            "shut-down" => run_status(
                apple_script("tell application \"System Events\" to shut down"),
                "shut down Mac",
            )?,
            _ => return Err("unknown system command".into()),
        };
        Ok(Some(message.into()))
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

fn run_status(status: io::Result<ExitStatus>, action: &str) -> Result<&'static str, String> {
    let status = status.map_err(|error| format!("could not {action}: {error}"))?;
    if status.success() {
        Ok("System command sent")
    } else {
        Err(format!("could not {action}: command exited with {status}"))
    }
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
        "mute" => "speaker.slash",
        "volume-up" => "speaker.plus",
        "volume-down" => "speaker.minus",
        "play-pause" => "playpause",
        "previous-track" => "backward.end",
        "next-track" => "forward.end",
        "eject-all" => "eject",
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
    fn includes_media_volume_and_disk_commands() {
        let ids: HashSet<_> = COMMANDS.iter().map(|command| command.id).collect();
        for id in [
            "mute",
            "volume-up",
            "volume-down",
            "play-pause",
            "previous-track",
            "next-track",
            "eject-all",
        ] {
            assert!(ids.contains(id), "missing command: {id}");
        }
        assert!(matches_query(
            COMMANDS
                .iter()
                .find(|command| command.id == "play-pause")
                .unwrap(),
            "播放"
        ));
        assert!(matches_query(
            COMMANDS
                .iter()
                .find(|command| command.id == "eject-all")
                .unwrap(),
            "弹出 磁盘"
        ));
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
