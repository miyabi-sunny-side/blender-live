//! JSON lines exchanged with the bridge script inside Blender.
use rmcp::schemars;
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "lowercase")]
pub enum Engine {
    Cycles,
    Eevee,
    Workbench,
}

impl Engine {
    /// The `scene.render.engine` identifier in Blender 5.
    #[must_use]
    pub fn identifier(self) -> &'static str {
        match self {
            Self::Cycles => "CYCLES",
            Self::Eevee => "BLENDER_EEVEE",
            Self::Workbench => "BLENDER_WORKBENCH",
        }
    }
}

impl Serialize for Engine {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.identifier())
    }
}

/// Camera, frame and output settings for one render. Unset fields keep the
/// scene's own values.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Shot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub width: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub height: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub engine: Option<Engine>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub samples: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Exec {
        code: String,
        timeout_secs: u64,
    },
    RunScript {
        path: String,
        timeout_secs: u64,
    },
    Summary,
    Open {
        path: String,
    },
    New {
        empty: bool,
    },
    Save {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
    },
    RenderStill {
        output: String,
        #[serde(flatten)]
        shot: Shot,
    },
    RenderFrames {
        dir: String,
        frame_start: i32,
        frame_end: i32,
        #[serde(flatten)]
        shot: Shot,
    },
    Cancel,
}

impl Op {
    /// How the op leaves the in-memory scene compared with its file:
    /// `Some(true)` changed, `Some(false)` matches a file, `None` untouched.
    #[must_use]
    pub fn unsaved_after(&self) -> Option<bool> {
        match self {
            // Renders set the camera, frame and output settings they use.
            Self::Exec { .. }
            | Self::RunScript { .. }
            | Self::RenderStill { .. }
            | Self::RenderFrames { .. } => Some(true),
            Self::Open { .. } | Self::New { .. } | Self::Save { .. } => Some(false),
            Self::Summary | Self::Cancel => None,
        }
    }
}

/// One request line, terminated by `\n`.
#[must_use]
pub fn request_line(id: u64, op: &Op) -> String {
    #[derive(Serialize)]
    struct Request<'a> {
        id: u64,
        #[serde(flatten)]
        op: &'a Op,
    }
    let mut line = serde_json::to_string(&Request { id, op }).expect("requests serialize");
    line.push('\n');
    line
}

#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Message {
    Ready {
        version: String,
        #[serde(default)]
        devices: Vec<String>,
        #[serde(default)]
        device_type: Option<String>,
    },
    Progress {
        id: u64,
        done: u32,
        total: u32,
    },
    Reply(Reply),
}

#[derive(Debug, PartialEq, Deserialize)]
pub struct Reply {
    pub id: u64,
    pub ok: bool,
    #[serde(default)]
    pub value: Value,
    #[serde(default)]
    pub stdout: String,
    #[serde(default)]
    pub error: Option<String>,
    #[serde(default)]
    pub traceback: Option<String>,
    /// The bridge stopped a frame render on request.
    #[serde(default)]
    pub cancelled: bool,
    /// `bpy.data.filepath` after the request; `None` for an unsaved scene.
    #[serde(default)]
    pub file: Option<String>,
}

pub fn parse_message(line: &str) -> Result<Message, serde_json::Error> {
    serde_json::from_str(line)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn wire(id: u64, op: &Op) -> Value {
        let line = request_line(id, op);
        assert!(line.ends_with('\n'), "line is not terminated: {line:?}");
        assert_eq!(line.matches('\n').count(), 1, "embedded newline: {line:?}");
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn engines_map_to_blender_identifiers() {
        assert_eq!(Engine::Cycles.identifier(), "CYCLES");
        assert_eq!(Engine::Eevee.identifier(), "BLENDER_EEVEE");
        assert_eq!(Engine::Workbench.identifier(), "BLENDER_WORKBENCH");
    }

    #[test]
    fn exec_requests_keep_multiline_code_on_one_line() {
        let op = Op::Exec {
            code: "import bpy\nprint(1)\n".into(),
            timeout_secs: 30,
        };
        assert_eq!(
            wire(7, &op),
            json!({"id": 7, "op": "exec", "code": "import bpy\nprint(1)\n", "timeout_secs": 30})
        );
    }

    #[test]
    fn render_requests_flatten_only_the_set_shot_fields() {
        let op = Op::RenderStill {
            output: "/camp/renders/a.png".into(),
            shot: Shot {
                engine: Some(Engine::Eevee),
                width: Some(640),
                ..Shot::default()
            },
        };
        assert_eq!(
            wire(1, &op),
            json!({"id": 1, "op": "render_still", "output": "/camp/renders/a.png",
                   "engine": "BLENDER_EEVEE", "width": 640})
        );
        let op = Op::RenderFrames {
            dir: "/camp/renders/clip".into(),
            frame_start: 1,
            frame_end: 48,
            shot: Shot::default(),
        };
        assert_eq!(
            wire(2, &op),
            json!({"id": 2, "op": "render_frames", "dir": "/camp/renders/clip",
                   "frame_start": 1, "frame_end": 48})
        );
    }

    #[test]
    fn save_without_a_path_saves_in_place() {
        assert_eq!(
            wire(3, &Op::Save { path: None }),
            json!({"id": 3, "op": "save"})
        );
        assert_eq!(wire(4, &Op::Cancel), json!({"id": 4, "op": "cancel"}));
    }

    #[test]
    fn unsaved_state_follows_what_each_op_does_to_the_scene() {
        let shot = Shot::default;
        for (op, expected) in [
            (
                Op::Exec {
                    code: String::new(),
                    timeout_secs: 1,
                },
                Some(true),
            ),
            (
                Op::RunScript {
                    path: String::new(),
                    timeout_secs: 1,
                },
                Some(true),
            ),
            (
                Op::RenderStill {
                    output: String::new(),
                    shot: shot(),
                },
                Some(true),
            ),
            (
                Op::Open {
                    path: String::new(),
                },
                Some(false),
            ),
            (Op::New { empty: true }, Some(false)),
            (Op::Save { path: None }, Some(false)),
            (Op::Summary, None),
            (Op::Cancel, None),
        ] {
            assert_eq!(op.unsaved_after(), expected, "{op:?}");
        }
    }

    #[test]
    fn parses_ready_progress_and_replies() {
        assert_eq!(
            parse_message(
                r#"{"type":"ready","version":"5.2.2","devices":["RTX"],"device_type":"OPTIX"}"#
            )
            .unwrap(),
            Message::Ready {
                version: "5.2.2".into(),
                devices: vec!["RTX".into()],
                device_type: Some("OPTIX".into()),
            }
        );
        assert_eq!(
            parse_message(r#"{"type":"progress","id":5,"done":3,"total":48}"#).unwrap(),
            Message::Progress {
                id: 5,
                done: 3,
                total: 48
            }
        );
        assert_eq!(
            parse_message(
                r#"{"type":"reply","id":5,"ok":false,"stdout":"hi\n","error":"ZeroDivisionError: division by zero","traceback":"Traceback..."}"#
            )
            .unwrap(),
            Message::Reply(Reply {
                id: 5,
                ok: false,
                value: Value::Null,
                stdout: "hi\n".into(),
                error: Some("ZeroDivisionError: division by zero".into()),
                traceback: Some("Traceback...".into()),
                cancelled: false,
                file: None,
            })
        );
    }

    #[test]
    fn rejects_lines_that_are_not_bridge_messages() {
        assert!(parse_message("Blender 5.2.2 LTS").is_err(), "accepted log");
        assert!(
            parse_message(r#"{"type":"other"}"#).is_err(),
            "accepted type"
        );
    }
}
