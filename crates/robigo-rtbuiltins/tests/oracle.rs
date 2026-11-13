//! The builtins against GNU R 4.6.2: every case in `tests/oracle/*.out`
//! (recorded by `tests/oracle/record.sh` with the 4.6.2 oracle image, never
//! the host's R) is replayed through the registry: its inputs decoded, its
//! call built, the builtin called, and the value, warnings (in order, with
//! the call each reports) and error compared with GNU R's, the doubles bit
//! for bit (NA against NaN) and the strings byte for byte with their
//! encoding mark. The format is described in `tests/oracle/helpers.R`.

mod common;

use common::{Core, V, decode, enc, hex, make};
use robigo_rtbuiltins::registry;
use robigo_rtcore::types::*;
use robigo_rtcore::{
    Args, Char, Cx, Dispatched, Generic, Hooks, Locale, Options, Out, OwnedIsolate, RResult, Ref, Root,
};

// ---- the cases --------------------------------------------------------------

struct Case {
    tag: String,
    call: String,
    inputs: Vec<(String, String)>,
    want: Vec<String>,
}

fn cases(text: &str) -> Vec<Case> {
    let mut out: Vec<Case> = Vec::new();
    for line in text.lines() {
        let (tag, rest) = line.split_once(' ').unwrap();
        if out.last().is_none_or(|c| c.tag != tag) {
            out.push(Case { tag: tag.to_owned(), call: String::new(), inputs: Vec::new(), want: Vec::new() });
        }
        let c = out.last_mut().unwrap();
        if let Some(v) = rest.strip_prefix("call : ") {
            c.call = v.to_owned();
        } else if let Some(v) = rest.strip_prefix("in ") {
            let (n, v) = v.split_once(" : ").unwrap();
            c.inputs.push((n.to_owned(), v.to_owned()));
        } else {
            c.want.push(rest.to_owned());
        }
    }
    out
}

/// A dispatcher with no methods: every internal generic runs its default.
fn no_methods<'o>(
    _: &mut Cx,
    _: &'static Generic,
    _: Root<'_>,
    _: Args<'_>,
    _: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    Ok(Dispatched::Default(out))
}

/// Run one case: the outcome lines in the recorded format.
fn run(cx: &mut Cx, case: &Case) -> Vec<String> {
    let call_v = decode(&case.call);
    let Core::Lang(parts) = &call_v.core else { panic!("{}: not a call", case.tag) };
    let Core::Sym(fname) = &parts[0].core else { panic!("{}: no function name", case.tag) };
    let fname = String::from_utf8(fname.clone()).unwrap();
    let b = registry::by_name(&fname).unwrap_or_else(|| panic!("{}: no builtin {fname}", case.tag));
    let inputs: Vec<(String, V)> = case.inputs.iter().map(|(n, v)| (n.clone(), decode(v))).collect();
    cx.scope(|cx, s| {
        let o = s.out(cx);
        let call = o.root();
        cx.build(o, |cx| make(cx, &call_v)).unwrap();
        let mut roots = Vec::new();
        for a in &parts[1..] {
            let Core::Sym(n) = &a.core else { panic!("{}: an argument that is not a symbol", case.tag) };
            let v = &inputs.iter().find(|(m, _)| m.as_bytes() == n.as_slice()).unwrap().1;
            let o = s.out(cx);
            roots.push(o.root());
            cx.build(o, |cx| make(cx, v)).unwrap();
        }
        let refs: Vec<Ref<'_>> = roots.iter().map(|r| r.bind(cx)).collect();
        let args = s.args(cx, &refs, None);
        let env = s.root(cx, cx.global_env());
        let res = s.out(cx);
        let res_root = res.root();
        let r = b.invoke(cx, call, args, env, res);
        let who = |cx: &Cx, c: Ref<'_>| {
            if c.sexp() == call.bind(cx).sexp() {
                "call"
            } else if c.sexptype() == NILSXP {
                "ctx"
            } else {
                "other"
            }
        };
        let mut lines: Vec<String> =
            cx.collected_warnings().iter().map(|(c, m)| format!("W : {} {}", hex(m.bytes()), who(cx, *c))).collect();
        cx.clear_warnings();
        match r {
            Ok(()) => lines.push(format!("= {}", enc(cx, res_root.bind(cx)))),
            Err(e) => {
                let w = who(cx, cx.exit_call(&e)).to_owned();
                let o = s.out(cx);
                let m = o.root();
                cx.resolve_exit(e, Some(o));
                let msg = m.bind(cx).as_vector::<Char<'_>>().map(|v| v.get(0).bytes().to_vec()).unwrap_or_default();
                lines.push(format!("E : {} {w}", hex(&msg)));
            }
        }
        lines
    })
}

/// Replay every case of `name.out`; the C-collation cases (tags starting
/// "C.") run with `LC_COLLATE=C`, as the script sets it. `not_yet` lists
/// the cases robigo answers with its "not implemented yet" error instead
/// (AGENTS rule 6): exactly those, so a new gap fails as surely as a wrong
/// value, and a closed one asks for its removal.
fn replay(name: &str, text: &str, not_yet: &[&str]) {
    replay_with(name, text, not_yet, Hooks { dispatch: no_methods, ..Hooks::default() });
}

fn replay_with(name: &str, text: &str, not_yet: &[&str], hooks: Hooks) {
    let mut iso = OwnedIsolate::with_options(Options { value_stack_slots: 1 << 16, ..Options::default() });
    let cx = iso.cx();
    cx.install_hooks(hooks);
    let all = cases(text);
    let mut bad = Vec::new();
    let mut gaps = Vec::new();
    // "robigo: " ... " is not implemented yet", hex-encoded in an E line
    let (head, tail) = (hex(b"robigo: "), hex(b" is not implemented yet"));
    for c in &all {
        let c_collate = c.tag.starts_with("C.");
        cx.set_locale(Locale { collate_c: c_collate, ..Locale::default() });
        let got = run(cx, c);
        let gap = got.last().is_some_and(|l| l.starts_with(&format!("E : {head}")) && l.contains(&tail));
        if gap {
            gaps.push(c.tag.clone());
        } else if got != c.want {
            bad.push(format!("{}\n  call {}\n  want {:?}\n  got  {:?}", c.tag, c.call, c.want, got));
        }
    }
    assert_eq!(gaps, not_yet, "{name}: the cases robigo does not implement yet");
    assert!(
        bad.is_empty(),
        "{name}: {} of {} cases differ from GNU R 4.6.2:\n{}",
        bad.len(),
        all.len(),
        bad.iter().take(400).cloned().collect::<Vec<_>>().join("\n")
    );
    eprintln!("{name}: {} cases match", all.len());
}

#[test]
fn arithmetic_matches_gnu_r() {
    replay("arith", include_str!("oracle/arith.out"), &[]);
}

#[test]
fn comparison_matches_gnu_r() {
    replay("relop", include_str!("oracle/relop.out"), &["bad.list"]);
}

#[test]
fn logic_matches_gnu_r() {
    replay("logic", include_str!("oracle/logic.out"), &[]);
}

#[test]
fn paste_matches_gnu_r() {
    replay("paste", include_str!("oracle/paste.out"), &[]);
}

#[test]
fn nchar_matches_gnu_r() {
    replay("nchar", include_str!("oracle/nchar.out"), &[]);
}

/// A full collection (a `Retry` safepoint with the isolate's roots).
fn collect(cx: &mut Cx) {
    cx.need_gc(robigo_rtcore::NeedGc::new(LGLSXP, 0)).unwrap();
}

/// The default warning path, after a collection.
fn warn_collecting(cx: &mut Cx, call: Option<Root<'_>>, msg: &str) -> RResult<()> {
    collect(cx);
    cx.collect_warning(call, msg);
    Ok(())
}

/// No methods, after a collection.
fn dispatch_collecting<'o>(
    cx: &mut Cx,
    _: &'static Generic,
    _: Root<'_>,
    _: Args<'_>,
    _: Root<'_>,
    out: Out<'o>,
) -> RResult<Dispatched<'o>> {
    collect(cx);
    Ok(Dispatched::Default(out))
}

/// Every case again with a full collection at every call a builtin makes
/// out of itself (each warning, each dispatch): what a builtin holds across
/// one must be rooted. Under `ROBIGO_GCTORTURE=1` (CI's rt job) freed
/// memory is poisoned, so a missed root shows as a wrong value.
#[test]
fn every_case_survives_collections_at_its_calls_out() {
    let h = Hooks { dispatch: dispatch_collecting, warning: warn_collecting, ..Hooks::default() };
    replay_with("arith", include_str!("oracle/arith.out"), &[], h);
    replay_with("relop", include_str!("oracle/relop.out"), &["bad.list"], h);
    replay_with("logic", include_str!("oracle/logic.out"), &[], h);
    replay_with("paste", include_str!("oracle/paste.out"), &[], h);
    replay_with("nchar", include_str!("oracle/nchar.out"), &[], h);
}
