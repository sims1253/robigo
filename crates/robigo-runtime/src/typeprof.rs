//! Runtime type profiling for comparison with static type checkers.
//!
//! With `ROBIGO_TYPE_PROFILE=out.json`, robigo records, for every closure call and
//! for the top level, the types of the bindings in the frame when it is
//! left (the same "scope exit" snapshot that `ry dump-facts` reports), and
//! writes them as JSON when the program ends. Function scopes are keyed by
//! the source position of their `function` keyword, like ry's scopes.

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::rc::Rc;

use crate::env::Env;
use crate::sym::s;
use crate::value::*;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct ObservedType {
    pub mode: String,
    /// Exact length if it was always the same, else None.
    pub length: Option<usize>,
    pub class: Vec<String>,
    pub attrs: Vec<String>,
    pub has_na: bool,
}

#[derive(Default)]
struct Profile {
    enabled: bool,
    /// Function body pointer -> (file, line, column) of `function`.
    spans: HashMap<usize, (Rc<str>, usize, usize)>,
    /// (file, line, col) or ("", 0, 0) for top level -> name -> types.
    scopes: BTreeMap<(String, usize, usize), BTreeMap<String, BTreeSet<ObservedType>>>,
    calls: BTreeMap<(String, usize, usize), usize>,
}

thread_local! {
    static PROFILE: RefCell<Profile> = RefCell::new(Profile {
        enabled: std::env::var_os("ROBIGO_TYPE_PROFILE").is_some(),
        ..Default::default()
    });
}

pub fn enabled() -> bool {
    PROFILE.with(|p| p.borrow().enabled)
}

/// Remember where a function literal's body came from.
pub fn record_function_span(body: &Value, file: &str, line: usize, col: usize) {
    if let Value::Lang(l) = body {
        PROFILE.with(|p| {
            p.borrow_mut().spans.insert(Rc::as_ptr(l) as usize, (Rc::from(file), line, col));
        });
    }
}

fn mode_of(v: &Value) -> String {
    match v {
        Value::Real(_) => "double".into(),
        Value::Closure(_) | Value::Builtin(_) => "function".into(),
        Value::Sym(_) | Value::Lang(_) => "language".into(),
        other => other.type_name().into(),
    }
}

fn observe(v: &Value) -> ObservedType {
    let has_na = match v {
        Value::Lgl(x) | Value::Int(x) => x.data.contains(&NA_INTEGER),
        Value::Real(x) => x.data.iter().any(|d| d.is_nan()),
        Value::Str(x) => x.data.iter().any(|d| d.is_none()),
        _ => false,
    };
    let class = match v.get_attr(s::CLASS) {
        Some(Value::Str(c)) => c.data.iter().map(|x| x.as_deref().unwrap_or("NA").to_string()).collect(),
        _ => vec![],
    };
    let attrs = v.attrs().map(|a| a.0.iter().map(|(k, _)| k.name().to_string()).collect()).unwrap_or_default();
    let length = match v {
        Value::Closure(_) | Value::Builtin(_) | Value::Env(_) => Some(1),
        other => Some(other.len()),
    };
    ObservedType { mode: mode_of(v), length, class, attrs, has_na }
}

/// Snapshot the bindings of a frame at scope exit.
pub fn record_frame(function_body: Option<&Value>, env: &Env) {
    let key = match function_body {
        None => (String::new(), 0, 0),
        Some(Value::Lang(l)) => match PROFILE.with(|p| p.borrow().spans.get(&(Rc::as_ptr(l) as usize)).cloned()) {
            Some((f, line, col)) => (f.to_string(), line, col),
            None => return,
        },
        Some(_) => return,
    };
    let bindings: Vec<(String, Value)> =
        env.0.frame().borrow().entries.iter().map(|b| (b.sym.name().to_string(), b.value.clone())).collect();
    PROFILE.with(|p| {
        let mut p = p.borrow_mut();
        *p.calls.entry(key.clone()).or_default() += 1;
        let scope = p.scopes.entry(key).or_default();
        for (name, v) in bindings {
            let v = match v {
                Value::Promise(pr) => match pr.value() {
                    Some(x) => x,
                    None => continue,
                },
                v if v.is_missing_arg() => continue,
                v => v,
            };
            scope.entry(name).or_default().insert(observe(&v));
        }
    });
}

fn json_str(s: &str) -> String {
    let mut o = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => o.push_str("\\\""),
            '\\' => o.push_str("\\\\"),
            '\n' => o.push_str("\\n"),
            c if (c as u32) < 0x20 => o.push_str(&format!("\\u{:04x}", c as u32)),
            c => o.push(c),
        }
    }
    o.push('"');
    o
}

/// Write the profile to the file named by `ROBIGO_TYPE_PROFILE`.
pub fn write() {
    let Some(path) = std::env::var_os("ROBIGO_TYPE_PROFILE") else { return };
    let text = PROFILE.with(|p| {
        let p = p.borrow();
        let mut out =
            String::from("{\n  \"producer\": \"robigo\",\n  \"snapshot_kind\": \"scope_exit\",\n  \"scopes\": [\n");
        let mut first_scope = true;
        for ((file, line, col), bindings) in &p.scopes {
            if !first_scope {
                out.push_str(",\n");
            }
            first_scope = false;
            let calls = p.calls.get(&(file.clone(), *line, *col)).copied().unwrap_or(0);
            if file.is_empty() {
                out.push_str("    {\"kind\": \"top\", \"start\": null");
            } else {
                out.push_str(&format!(
                    "    {{\"kind\": \"function\", \"file\": {}, \"start\": [{}, {}]",
                    json_str(file),
                    line,
                    col
                ));
            }
            out.push_str(&format!(", \"calls\": {}, \"bindings\": [\n", calls));
            let mut first = true;
            for (name, types) in bindings {
                if !first {
                    out.push_str(",\n");
                }
                first = false;
                out.push_str(&format!("      {{\"name\": {}, \"observed\": [", json_str(name)));
                let ts: Vec<String> = types
                    .iter()
                    .map(|t| {
                        format!(
                            "{{\"mode\": {}, \"length\": {}, \"class\": [{}], \"attributes\": [{}], \"has_na\": {}}}",
                            json_str(&t.mode),
                            t.length.map(|l| l.to_string()).unwrap_or_else(|| "null".into()),
                            t.class.iter().map(|c| json_str(c)).collect::<Vec<_>>().join(", "),
                            t.attrs.iter().map(|c| json_str(c)).collect::<Vec<_>>().join(", "),
                            t.has_na
                        )
                    })
                    .collect();
                out.push_str(&ts.join(", "));
                out.push_str("]}");
            }
            out.push_str("\n    ]}");
        }
        out.push_str("\n  ]\n}\n");
        out
    });
    let _ = std::fs::write(path, text);
}
