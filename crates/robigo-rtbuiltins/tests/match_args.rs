//! `util::match_args` (GNU R's `matchArgs_NR`): its fast path for the
//! common shape, and the inputs that must take the full matcher.

use robigo_rtbuiltins::util::match_args;
use robigo_rtcore::{Char, Cx, Options, OwnedIsolate};

/// `match_args(formals, args)` for arguments `1, 2, ...` tagged by `tags`:
/// the formal each argument went to, or the error's message.
fn matched(cx: &mut Cx, formals: &[&str], tags: &[Option<&str>]) -> Result<Vec<Option<usize>>, String> {
    cx.scope(|cx, s| {
        let call = s.root(cx, cx.nil());
        let mut vals = Vec::new();
        for i in 0..tags.len() {
            let o = s.out(cx);
            let r = o.root();
            cx.build(o, |cx| Ok(cx.alloc_copy(&[i as f64 + 1.0])?.finish().into())).unwrap();
            vals.push(r);
        }
        let names: Vec<_> =
            tags.iter().map(|t| t.map(|t| cx.install(t.as_bytes()).unwrap().as_ref())).collect::<Vec<_>>();
        let refs: Vec<_> = vals.iter().map(|r| r.bind(cx)).collect();
        let syms: Vec<_> = names.iter().map(|n| n.and_then(|n| n.as_sym())).collect();
        let args = s.args(cx, &refs, Some(&syms));
        match match_args(cx, call, formals, args) {
            Ok(m) => Ok(m.formals),
            Err(e) => {
                let o = s.out(cx);
                let m = o.root();
                cx.resolve_exit(e, Some(o));
                Err(m
                    .bind(cx)
                    .as_vector::<Char<'_>>()
                    .map(|v| String::from_utf8_lossy(v.get(0).bytes()).into_owned())
                    .unwrap_or_default())
            }
        }
    })
}

#[test]
fn match_args_fast_path_and_repeated_formals() {
    let mut iso = OwnedIsolate::with_options(Options::default());
    let cx = iso.cx();
    // attr<-'s shape: positional, positional, `value =`
    assert_eq!(
        matched(cx, &["x", "which", "value"], &[None, None, Some("value")]),
        Ok(vec![Some(0), Some(1), Some(2)])
    );
    // fewer arguments than formals, `...` after them
    assert_eq!(matched(cx, &["x", "y", "..."], &[None, Some("y")]), Ok(vec![Some(0), Some(1), None]));
    // a closure's formals may repeat a name (`formals(f) <- alist(x=, x=)`):
    // GNU R 4.6.2's match.call(f, quote(f(x = 1))) gives this error
    assert_eq!(matched(cx, &["x", "x"], &[Some("x")]), Err("argument 1 matches multiple formal arguments".to_owned()));
}
