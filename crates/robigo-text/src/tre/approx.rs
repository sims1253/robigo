//! TRE's approximate matcher (tre-match-approx.c): the TNFA simulation
//! that also tries inserting, deleting and substituting characters, within
//! the costs and limits that `{~n}`-style parameters set.
//!
//! Plain `regexec` runs it for every regex with such parameters, with a
//! maximum cost and error count of zero outside them.
//!
//! The port keeps C's aliasing: when a delete loops a state back onto
//! itself, the later transitions of that state read the entry it just
//! overwrote. Every read of the source entry below happens where C's does.

use super::Error;
use super::ast::{
    PARAM_COST_DEL, PARAM_COST_INS, PARAM_COST_MAX, PARAM_COST_SUBST, PARAM_DEFAULT, PARAM_DEPTH, PARAM_MAX_DEL,
    PARAM_MAX_ERR, PARAM_MAX_INS, PARAM_MAX_SUBST, PARAM_UNSET,
};
use super::exec::{Ctx, Cursor, Text, tag_order};

const M_COST: usize = 0;
const M_NUM_INS: usize = 1;
const M_NUM_DEL: usize = 2;
const M_NUM_SUBST: usize = 3;
const M_NUM_ERR: usize = 4;
const M_LAST: usize = 5;
const M_MAX_DEPTH: usize = 3;

/// `regaparams_t`: costs of an edit and the limits on a match.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AParams {
    pub cost_ins: i32,
    pub cost_del: i32,
    pub cost_subst: i32,
    pub max_cost: i32,
    pub max_ins: i32,
    pub max_del: i32,
    pub max_subst: i32,
    pub max_err: i32,
}

impl Default for AParams {
    // R: regexec.c:tre_regaparams_default
    fn default() -> AParams {
        AParams {
            cost_ins: 1,
            cost_del: 1,
            cost_subst: 1,
            max_cost: i32::MAX,
            max_ins: i32::MAX,
            max_del: i32::MAX,
            max_subst: i32::MAX,
            max_err: i32::MAX,
        }
    }
}

/// The costs of an approximate match (`regamatch_t` without `pmatch`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AMatch {
    pub cost: i32,
    pub num_ins: i32,
    pub num_del: i32,
    pub num_subst: i32,
}

/// `tre_tnfa_approx_reach_t` without its tags, which live in a table of
/// their own.
#[derive(Clone, Copy)]
struct AReach {
    /// The state: the index of its first transition.
    state: usize,
    pos: i32,
    params: AParams,
    depth: usize,
    costs: [[i32; M_LAST]; M_MAX_DEPTH + 1],
}

/// A reach table: one entry and one tag array per state.
struct Table {
    e: Vec<AReach>,
    tags: Vec<i32>,
}

/// Sets `r`'s parameters from a transition's, taking defaults from
/// `default_params`.
// R: tre-match-approx.c:tre_set_params
fn set_params(r: &mut AReach, pa: &[i32], default_params: &AParams) -> Result<(), Error> {
    // A deeper nesting starts the new levels' costs and counters at zero.
    let value = pa[PARAM_DEPTH];
    if value > M_MAX_DEPTH as i32 {
        return Err(Error::Assert { expr: "value <= 3", file: "tre-match-approx.c", line: 141 });
    }
    let value = value as usize;
    if value > r.depth {
        for level in &mut r.costs[r.depth + 1..=value] {
            *level = [0; M_LAST];
        }
    }
    r.depth = value;

    let p = &mut r.params;
    let set = |field: &mut i32, value: i32, default: i32| {
        if value == PARAM_DEFAULT {
            *field = default;
        } else if value != PARAM_UNSET {
            *field = value;
        }
    };
    set(&mut p.cost_ins, pa[PARAM_COST_INS], default_params.cost_ins);
    set(&mut p.cost_del, pa[PARAM_COST_DEL], default_params.cost_del);
    // C sets the substitute cost even when unset.
    let value = pa[PARAM_COST_SUBST];
    p.cost_subst = if value == PARAM_DEFAULT { default_params.cost_subst } else { value };
    set(&mut p.max_cost, pa[PARAM_COST_MAX], default_params.max_cost);
    set(&mut p.max_ins, pa[PARAM_MAX_INS], default_params.max_ins);
    set(&mut p.max_del, pa[PARAM_MAX_DEL], default_params.max_del);
    set(&mut p.max_subst, pa[PARAM_MAX_SUBST], default_params.max_subst);
    set(&mut p.max_err, pa[PARAM_MAX_ERR], default_params.max_err);
    Ok(())
}

/// The ring buffer of the delete pass: C's array of `reach_next` entries,
/// 256 long and grown by 512 only while it is first filled, so that a
/// pass that queues more states than it holds fails C's assertion.
struct Deque {
    buf: Vec<usize>,
    start: usize,
    end: usize,
}

impl Deque {
    fn new() -> Deque {
        Deque { buf: vec![0; 256], start: 0, end: 0 }
    }

    /// Appends during the first fill, growing the buffer.
    fn fill(&mut self, id: usize) {
        self.buf[self.end] = id;
        self.end += 1;
        if self.end >= self.buf.len() {
            let n = self.buf.len() + 512;
            self.buf.resize(n, 0);
        }
    }

    fn push(&mut self, id: usize) -> Result<(), Error> {
        self.buf[self.end] = id;
        self.end += 1;
        if self.end >= self.buf.len() {
            self.end = 0;
        }
        if self.end == self.start {
            return Err(Error::Assert { expr: "deque_end != deque_start", file: "tre-match-approx.c", line: 649 });
        }
        Ok(())
    }

    fn pop_front(&mut self) {
        self.start += 1;
        if self.start >= self.buf.len() {
            self.start = 0;
        }
    }
}

/// Runs the TNFA over `s` allowing edits; the best match's end (or -1)
/// and its costs. `match_tags` is None when the caller wants no
/// submatches.
// R: tre-match-approx.c:tre_tnfa_run_approx
pub(super) fn run_approx<T: Text + ?Sized>(
    ctx: &Ctx,
    s: &T,
    len: i32,
    match_tags: Option<&mut [i32]>,
    default_params: &AParams,
) -> Result<(i32, AMatch), Error> {
    let tnfa = ctx.tnfa;
    let mut no_tags: [i32; 0] = [];
    let (mt, nt) = match match_tags {
        Some(m) => (m, tnfa.num_tags as usize),
        None => (&mut no_tags[..], 0),
    };
    let have_tags = nt > 0;
    let n = tnfa.num_states as usize;
    let dirs = &tnfa.tag_directions[..];
    let trans = &tnfa.transitions[..];
    let blank = AReach {
        state: 0,
        pos: -2,
        params: AParams {
            cost_ins: 0,
            cost_del: 0,
            cost_subst: 0,
            max_cost: 0,
            max_ins: 0,
            max_del: 0,
            max_subst: 0,
            max_err: 0,
        },
        depth: 0,
        costs: [[0; M_LAST]; M_MAX_DEPTH + 1],
    };
    let mut reach = Table { e: vec![blank; n], tags: vec![0; n * nt] };
    let mut reach_next = Table { e: vec![blank; n], tags: vec![0; n * nt] };
    let mut tmp_tags = vec![0i32; nt];

    let mut match_eo = -1;
    let mut match_costs = [i32::MAX; M_LAST];

    let mut cur = Cursor::new(-1);
    let mut prev_pos = cur.pos;
    if !cur.next(s, len) {
        return Ok((-1, AMatch::default()));
    }
    cur.pos = 0;

    loop {
        let pos = cur.pos;
        // Add the initial states to `reach_next` unless an exact match
        // has been found.
        if match_costs[M_COST] > 0 {
            for t in &tnfa.initial {
                let id = t.state_id as usize;
                if reach_next.e[id].pos >= pos {
                    continue;
                }
                if t.assertions != 0 && ctx.assertions_fail(t.assertions, &cur) {
                    continue;
                }
                let r = &mut reach_next.e[id];
                r.state = t.state.expect("initial transition without a state");
                r.pos = pos;
                let tags = &mut reach_next.tags[id * nt..(id + 1) * nt];
                tags.fill(-1);
                for &tg in &t.tags {
                    if (tg as usize) < nt {
                        tags[tg as usize] = pos;
                    }
                }
                r.params = *default_params;
                r.depth = 0;
                r.costs[0] = [0; M_LAST];
                if let Some(p) = &t.params {
                    set_params(r, &p[..], default_params)?;
                }
                if r.state == tnfa.final_state {
                    match_eo = pos;
                    mt.copy_from_slice(tags);
                    match_costs = [0; M_LAST];
                }
            }
        }

        // Inserts: an epsilon transition from each state in `reach` back
        // to itself. These never improve on a match already found.
        for id in 0..n {
            let r = reach.e[id];
            if r.pos != prev_pos {
                continue;
            }
            let depth = r.depth;
            let mut cost = r.costs[depth][M_COST];
            if r.params.cost_ins != PARAM_UNSET {
                cost = cost.wrapping_add(r.params.cost_ins);
            }
            if cost > r.params.max_cost
                || r.costs[depth][M_NUM_INS].wrapping_add(1) > r.params.max_ins
                || r.costs[depth][M_NUM_ERR].wrapping_add(1) > r.params.max_err
            {
                continue;
            }
            let mut cost0 = cost;
            if depth > 0 {
                cost0 = r.costs[0][M_COST].wrapping_add(if r.params.cost_ins != PARAM_UNSET {
                    r.params.cost_ins
                } else {
                    default_params.cost_ins
                });
            }
            let rn = &mut reach_next.e[id];
            if rn.pos == pos && cost0 >= rn.costs[0][M_COST] {
                continue;
            }
            rn.state = r.state;
            rn.pos = pos;
            reach_next.tags[id * nt..(id + 1) * nt].copy_from_slice(&reach.tags[id * nt..(id + 1) * nt]);
            rn.params = r.params;
            rn.depth = r.depth;
            rn.costs[..=depth].copy_from_slice(&r.costs[..=depth]);
            rn.costs[depth][M_COST] = cost;
            rn.costs[depth][M_NUM_INS] += 1;
            rn.costs[depth][M_NUM_ERR] += 1;
            if depth > 0 {
                rn.costs[0][M_COST] = cost0;
                rn.costs[0][M_NUM_INS] += 1;
                rn.costs[0][M_NUM_ERR] += 1;
            }
        }

        // Deletes: follow every transition of the TNFA as if it were an
        // epsilon transition until no state can be reached more cheaply.
        let mut dq = Deque::new();
        for id in 0..n {
            if reach_next.e[id].pos == pos {
                dq.fill(id);
            }
        }
        while dq.end != dq.start {
            let id = dq.buf[dq.start];
            let src = reach_next.e[id];
            let depth = src.depth;
            let mut cost = src.costs[depth][M_COST];
            if src.params.cost_del != PARAM_UNSET {
                cost = cost.wrapping_add(src.params.cost_del);
            }
            if cost > src.params.max_cost
                || src.costs[depth][M_NUM_DEL].wrapping_add(1) > src.params.max_del
                || src.costs[depth][M_NUM_ERR].wrapping_add(1) > src.params.max_err
            {
                dq.pop_front();
                continue;
            }
            let mut cost0 = cost;
            if depth > 0 {
                cost0 = src.costs[0][M_COST].wrapping_add(if src.params.cost_del != PARAM_UNSET {
                    src.params.cost_del
                } else {
                    default_params.cost_del
                });
            }

            let mut ti = src.state;
            while trans[ti].state.is_some() {
                let t = &trans[ti];
                ti += 1;
                if t.assertions != 0 && ctx.assertions_fail(t.assertions, &cur) {
                    continue;
                }
                // The source entry as it is now: an earlier transition of
                // this loop may have overwritten it.
                tmp_tags.copy_from_slice(&reach_next.tags[id * nt..(id + 1) * nt]);
                for &tg in &t.tags {
                    if (tg as usize) < nt {
                        tmp_tags[tg as usize] = pos;
                    }
                }
                let d = t.state_id as usize;
                let dest = reach_next.e[d];
                if dest.pos == pos
                    && (cost0 > dest.costs[0][M_COST]
                        || (cost0 == dest.costs[0][M_COST]
                            && (!have_tags || !tag_order(nt, dirs, &tmp_tags, &reach_next.tags[d * nt..(d + 1) * nt]))))
                {
                    continue;
                }
                let dstate = t.state.unwrap();
                reach_next.e[d].state = dstate;
                reach_next.e[d].pos = pos;
                reach_next.tags[d * nt..(d + 1) * nt].copy_from_slice(&tmp_tags);
                reach_next.e[d].params = reach_next.e[id].params;
                if let Some(p) = &t.params {
                    set_params(&mut reach_next.e[d], &p[..], default_params)?;
                }
                reach_next.e[d].depth = reach_next.e[id].depth;
                let costs = reach_next.e[id].costs;
                let rd = &mut reach_next.e[d];
                rd.costs[..=depth].copy_from_slice(&costs[..=depth]);
                rd.costs[depth][M_COST] = cost;
                rd.costs[depth][M_NUM_DEL] += 1;
                rd.costs[depth][M_NUM_ERR] += 1;
                if depth > 0 {
                    rd.costs[0][M_COST] = cost0;
                    rd.costs[0][M_NUM_DEL] += 1;
                    rd.costs[0][M_NUM_ERR] += 1;
                }
                if dstate == tnfa.final_state
                    && (match_eo < 0
                        || match_costs[M_COST] > cost0
                        || (match_costs[M_COST] == cost0 && nt > 0 && tmp_tags[0] <= mt[0]))
                {
                    match_eo = pos;
                    match_costs = rd.costs[0];
                    mt.copy_from_slice(&tmp_tags);
                }
                dq.push(d)?;
            }
            dq.pop_front();
        }

        if cur.at_end(len) {
            break;
        }
        prev_pos = pos;
        if !cur.next(s, len) {
            return Ok((-1, AMatch::default()));
        }
        let pos = cur.pos;
        std::mem::swap(&mut reach, &mut reach_next);

        // Exact matches and substitutes.
        let prev_c = cur.prev_c as u32;
        for id in 0..n {
            let r = reach.e[id];
            if r.pos < prev_pos {
                continue;
            }
            let mut ti = r.state;
            while trans[ti].state.is_some() {
                let t = &trans[ti];
                ti += 1;
                if t.assertions != 0 && (ctx.assertions_fail(t.assertions, &cur) || ctx.classes_fail(t, cur.prev_c)) {
                    continue;
                }
                let depth = r.depth;
                let d = t.state_id as usize;
                let mut cost = r.costs[depth][M_COST];
                let mut cost0 = r.costs[0][M_COST];
                let mut err = 0;
                // `tre_cint_t` is unsigned.
                if (t.code_min as u32) > prev_c || (t.code_max as u32) < prev_c {
                    // A substitute: match the character the transition
                    // wants in place of the one in the string.
                    err = 1;
                    if r.params.cost_subst != PARAM_UNSET {
                        cost = cost.wrapping_add(r.params.cost_subst);
                    }
                    if cost > r.params.max_cost
                        || r.costs[depth][M_NUM_SUBST].wrapping_add(1) > r.params.max_subst
                        || r.costs[depth][M_NUM_ERR].wrapping_add(1) > r.params.max_err
                    {
                        continue;
                    }
                    cost0 = cost;
                    if depth > 0 {
                        cost0 = r.costs[0][M_COST].wrapping_add(if r.params.cost_subst != PARAM_UNSET {
                            r.params.cost_subst
                        } else {
                            default_params.cost_subst
                        });
                    }
                }
                tmp_tags.copy_from_slice(&reach.tags[id * nt..(id + 1) * nt]);
                for &tg in &t.tags {
                    if (tg as usize) < nt {
                        tmp_tags[tg as usize] = pos;
                    }
                }
                let dest = &reach_next.e[d];
                if dest.pos == pos
                    && (cost0 > dest.costs[0][M_COST]
                        || (cost0 == dest.costs[0][M_COST]
                            && !tag_order(nt, dirs, &tmp_tags, &reach_next.tags[d * nt..(d + 1) * nt])))
                {
                    continue;
                }
                let dstate = t.state.unwrap();
                let rd = &mut reach_next.e[d];
                rd.state = dstate;
                rd.pos = pos;
                reach_next.tags[d * nt..(d + 1) * nt].copy_from_slice(&tmp_tags);
                rd.depth = r.depth;
                rd.params = r.params;
                if let Some(p) = &t.params {
                    set_params(rd, &p[..], default_params)?;
                }
                rd.costs[..=depth].copy_from_slice(&r.costs[..=depth]);
                rd.costs[depth][M_COST] = cost;
                rd.costs[depth][M_NUM_SUBST] += err;
                rd.costs[depth][M_NUM_ERR] += err;
                if depth > 0 {
                    rd.costs[0][M_COST] = cost0;
                    rd.costs[0][M_NUM_SUBST] += err;
                    rd.costs[0][M_NUM_ERR] += err;
                }
                if dstate == tnfa.final_state
                    && (match_eo < 0
                        || cost0 < match_costs[M_COST]
                        || (cost0 == match_costs[M_COST] && nt > 0 && tmp_tags[0] <= mt[0]))
                {
                    match_eo = pos;
                    match_costs = rd.costs[0];
                    mt.copy_from_slice(&tmp_tags);
                }
            }
        }
    }

    let m = AMatch {
        cost: match_costs[M_COST],
        num_ins: match_costs[M_NUM_INS],
        num_del: match_costs[M_NUM_DEL],
        num_subst: match_costs[M_NUM_SUBST],
    };
    Ok((match_eo, m))
}
