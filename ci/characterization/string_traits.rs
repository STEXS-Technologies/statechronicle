//! External old-compatible string trait and wire characterization.
use serde_json::{Value, json};
use statechronicle::domain::{
    commit::ProfileId,
    ids::{CommitId, EventId, IntentId, SnapshotId, StateId},
    intent::{KeyId, Operation},
    status::Status,
};
use std::fmt::{Debug, Display};

fn outcome<T: Debug + Display, E: Debug + Display>(result: Result<T, E>) -> Value {
    match result {
        Ok(value) => json!({"ok": format!("{value:?}"), "display": value.to_string()}),
        Err(error) => json!({"error": format!("{error:?}"), "display": error.to_string()}),
    }
}
fn inputs(prefix: &str) -> Vec<String> {
    let mut values = vec![
        String::new(),
        prefix.to_owned(),
        format!("{prefix}a"),
        String::from("wrong_x"),
        String::from("STC_x"),
        String::from(" "),
        String::from("a b"),
        String::from("a:b#key"),
        String::from("a.b.c"),
        String::from("stc_x"),
        String::from("int_x"),
        String::from("evt_x"),
        String::from("cmt_x"),
        String::from("snp_x"),
    ];
    for length in 0..=260 {
        for body in ["x", "é", "🦀", "\u{301}"] {
            values.push(format!("{prefix}{}", body.repeat(length)));
        }
    }
    for point in 0..=159 {
        if let Some(control) = char::from_u32(point).filter(|value| value.is_control()) {
            values.push(format!("{prefix}x{control}y"));
        }
    }
    for character in ['\u{2028}', '\u{2029}', '\u{200b}', '\u{feff}', '\u{10ffff}'] {
        values.push(format!("{prefix}{character}"));
    }
    values
}
macro_rules! characterize {
    ($rows:ident, $name:ident, $prefix:literal) => {
        characterize!($rows, $name, $prefix, $name);
    };
    ($rows:ident, $name:ident, $prefix:literal, $raw:expr) => {{
        for (index, value) in inputs($prefix).into_iter().enumerate() {
            // Public tuple construction is intentionally unchecked in the old API.
            let raw = ($raw)(value.clone());
            let bytes = bcs::to_bytes(&value).unwrap();
            let json = serde_json::to_string(&value).unwrap();
            $rows.push(json!({
                "case": format!("{}:string:{index}", stringify!($name)),
                "input": value,
                "new": outcome($name::new(value.clone())),
                "parse": outcome(value.parse::<$name>()),
                "owned": outcome($name::try_from(value.clone())),
                "borrowed": outcome($name::try_from(value.as_str())),
                "json_decode": outcome(serde_json::from_str::<$name>(&json)),
                "bcs_decode": outcome(bcs::from_bytes::<$name>(&bytes)),
                "raw_debug": format!("{raw:?}"),
                "raw_display": raw.to_string(),
                "raw_format": format!("{raw:>12.3}"),
                "raw_json": serde_json::to_string(&raw).unwrap(),
                "raw_bcs": bcs::to_bytes(&raw).unwrap(),
                "raw_canonical": statechronicle::core::canonicalize::canonicalize(&raw).unwrap(),
            }));
        }
        for (index, json) in ["null", "false", "0", "1.5", "[]", "{}", "[\"x\"]", "{\"0\":\"x\"}", "\"x\" true", "\"\\uD800\""].into_iter().enumerate() {
            $rows.push(json!({"case":format!("{}:json:{index}", stringify!($name)), "input":json, "result":outcome(serde_json::from_str::<$name>(json))}));
        }
        for (index, bytes) in [vec![], vec![0], vec![0x80, 0], vec![1, 0xff], vec![1,b'x',0], vec![0xff;5], vec![0x80;10]].into_iter().enumerate() {
            $rows.push(json!({"case":format!("{}:bcs:{index}", stringify!($name)), "input":bytes, "result":outcome(bcs::from_bytes::<$name>(&bytes))}));
        }
    }};
}
fn main() {
    let mut rows = Vec::new();
    characterize!(rows, StateId, "stc_");
    characterize!(rows, IntentId, "int_");
    characterize!(rows, EventId, "evt_");
    characterize!(rows, CommitId, "cmt_");
    characterize!(rows, SnapshotId, "snp_");
    characterize!(rows, ProfileId, "profile.");
    characterize!(rows, Operation, "op.");
    characterize!(rows, KeyId, "key:");
    // Status has a private tuple field, but its trusted static constructor
    // retains the same unchecked string behavior. Leaking here is test-only.
    characterize!(
        rows,
        Status,
        "status.",
        |value: String| Status::from_static(Box::leak(value.into_boxed_str()))
    );
    serde_json::to_writer(std::io::stdout(), &rows).unwrap();
}
