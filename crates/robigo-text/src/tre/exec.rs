//! TRE's matchers: the parallel TNFA simulation for regexes without back
//! references (tre-match-parallel.c), the backtracking matcher for those
//! with them (tre-match-backtrack.c), and the submatch bookkeeping of
//! regexec.c.
//!
//! All read the subject the way TRE's `GET_NEXT_WCHAR` does, and keep its
//! quirks: after a back reference of two or more characters `prev_c` is
//! the reference's first character, and the backtracking matcher stops
//! trying later start positions once a failed path has reached the end.

use super::approx::{AMatch, AParams, run_approx};
use super::compile::{TAG_MINIMIZE, Tnfa, Transition};
use super::ctype::{Class, tolower, toupper};
use super::parse::{
    ASSERT_AT_BOL, ASSERT_AT_BOW, ASSERT_AT_EOL, ASSERT_AT_EOW, ASSERT_AT_WB, ASSERT_AT_WB_NEG, ASSERT_BACKREF,
    ASSERT_CHAR_CLASS, ASSERT_CHAR_CLASS_NEG,
};
use super::{Error, REG_ICASE, REG_NEWLINE, REG_NOSUB, REG_NOTBOL, REG_NOTEOL};
use crate::Locale;

/// A subject string: bytes (`STR_BYTE`), wide characters (`STR_WIDE`) or
/// a multibyte string in the locale's encoding ([`Mbs`], `STR_MBS`).
/// Reading at or past the end gives C's terminating `'\0'`.
pub trait Text {
    /// `type == STR_BYTE`, for the first-character search.
    const BYTES: bool;
    /// `type == STR_MBS`: read with [`Text::decode`], positions in bytes.
    const MBS: bool = false;
    fn len(&self) -> usize;
    /// The character at `i`, or 0 outside the string.
    fn get(&self, i: isize) -> i32;
    /// The first `c` at or after `from`: `memchr` within `len`, or
    /// `strchr` (which finds the terminator for `c == 0`).
    fn find(&self, c: i32, len: i32) -> Option<usize>;
    /// `mbrtowc` of at most `max` bytes at `i`: the character and its
    /// width (0 for the terminator), or `None` for an invalid or
    /// incomplete sequence.
    fn decode(&self, _i: isize, _max: usize) -> Option<(i32, usize)> {
        unreachable!("decode on a fixed-width string")
    }
}

/// A UTF-8 string read as `STR_MBS`: what `tre_regexec` does in a UTF-8
/// locale, where R only calls it for `regexec(fixed = TRUE)` on native
/// strings.
pub struct Mbs<'a>(pub &'a [u8]);

impl Text for Mbs<'_> {
    const BYTES: bool = false;
    const MBS: bool = true;
    #[inline]
    fn len(&self) -> usize {
        self.0.len()
    }
    #[inline]
    fn get(&self, i: isize) -> i32 {
        <[u8] as Text>::get(self.0, i)
    }
    fn find(&self, _c: i32, _len: i32) -> Option<usize> {
        unreachable!("first-character search on a multibyte string")
    }
    // R: the locale's mbrtowc
    fn decode(&self, i: isize, max: usize) -> Option<(i32, usize)> {
        let start = (i.max(0) as usize).min(self.0.len());
        // At most `max` bytes: a character cut short is invalid.
        let end = start.saturating_add(max).min(self.0.len());
        match crate::mbrtowc_utf8(&self.0[..end], start) {
            (n, wc) if n >= 0 => Some((wc, n as usize)),
            _ => None,
        }
    }
}

impl Text for [u8] {
    const BYTES: bool = true;
    #[inline]
    fn len(&self) -> usize {
        <[u8]>::len(self)
    }
    #[inline]
    fn get(&self, i: isize) -> i32 {
        if i >= 0 && (i as usize) < <[u8]>::len(self) { self[i as usize] as i32 } else { 0 }
    }
    fn find(&self, c: i32, len: i32) -> Option<usize> {
        // memchr/strchr compare as unsigned char.
        let b = c as u8;
        if len >= 0 {
            let n = (len as usize).min(<[u8]>::len(self));
            return self[..n].iter().position(|&x| x == b);
        }
        // strchr: the byte or the string's NUL, whichever comes first (the
        // end of the slice stands for the NUL)
        match self.iter().position(|&x| x == b || x == 0) {
            Some(p) if self[p] == b => Some(p),
            Some(_) => None,
            None if b == 0 => Some(<[u8]>::len(self)),
            None => None,
        }
    }
}

impl Text for [i32] {
    const BYTES: bool = false;
    #[inline]
    fn len(&self) -> usize {
        <[i32]>::len(self)
    }
    #[inline]
    fn get(&self, i: isize) -> i32 {
        if i >= 0 && (i as usize) < <[i32]>::len(self) { self[i as usize] } else { 0 }
    }
    fn find(&self, _c: i32, _len: i32) -> Option<usize> {
        unreachable!("first-character search on a wide string")
    }
}

/// `strncmp`/`wcsncmp` (`nul_stop`) or `memcmp`/`wmemcmp` equality of
/// `n` characters at `a` and `b`.
fn ncmp_eq<T: Text + ?Sized>(s: &T, a: isize, b: isize, n: i32, nul_stop: bool) -> bool {
    for k in 0..n as isize {
        let (ca, cb) = (s.get(a + k), s.get(b + k));
        if ca != cb {
            return false;
        }
        if nul_stop && ca == 0 {
            return true;
        }
    }
    true
}

/// The reading state `GET_NEXT_WCHAR` advances: `str_byte`/`str_wide`
/// is `idx`, the index of the character after `next_c`.
pub(super) struct Cursor {
    pub(super) prev_c: i32,
    next_c: i32,
    pub(super) pos: i32,
    idx: isize,
    /// `STR_MBS`: the width of `next_c`, added to `pos` by the next read.
    pos_add_next: i32,
}

impl Cursor {
    pub(super) fn new(pos: i32) -> Cursor {
        Cursor { prev_c: 0, next_c: 0, pos, idx: 0, pos_add_next: 1 }
    }

    /// `false` where C's macro returns REG_NOMATCH (an invalid multibyte
    /// sequence).
    // R: tre-match-utils.h:GET_NEXT_WCHAR
    #[inline]
    #[must_use]
    pub(super) fn next<T: Text + ?Sized>(&mut self, s: &T, len: i32) -> bool {
        self.prev_c = self.next_c;
        if T::MBS {
            self.pos += self.pos_add_next;
            let max = if len >= 0 { len - self.pos } else { 32 };
            if max <= 0 {
                self.next_c = 0;
                self.pos_add_next = 1;
            } else {
                let Some((c, w)) = s.decode(self.idx, max as usize) else { return false };
                if w == 0 && len >= 0 {
                    self.pos_add_next = 1;
                    self.next_c = 0;
                    self.idx += 1;
                } else {
                    self.pos_add_next = w as i32;
                    self.next_c = c;
                    self.idx += w as isize;
                }
            }
            return true;
        }
        self.pos += 1;
        if len >= 0 && self.pos >= len {
            self.next_c = 0;
        } else {
            self.next_c = s.get(self.idx);
            self.idx += 1;
        }
        true
    }

    /// The end-of-string test both matchers make before reading.
    #[inline]
    pub(super) fn at_end(&self, len: i32) -> bool {
        if len < 0 { self.next_c == 0 } else { self.pos >= len }
    }
}

/// What the assertion and class checks need besides the cursor.
pub(super) struct Ctx<'t> {
    pub(super) tnfa: &'t Tnfa,
    notbol: bool,
    noteol: bool,
    newline: bool,
    icase: bool,
    locale: Locale,
}

impl Ctx<'_> {
    fn new(tnfa: &Tnfa, eflags: i32, locale: Locale) -> Ctx<'_> {
        Ctx {
            tnfa,
            notbol: eflags & REG_NOTBOL != 0,
            noteol: eflags & REG_NOTEOL != 0,
            newline: tnfa.cflags & REG_NEWLINE != 0,
            icase: tnfa.cflags & REG_ICASE != 0,
            locale,
        }
    }

    #[inline]
    fn isctype(&self, c: i32, class: Class) -> bool {
        class.contains(c as u32, self.locale)
    }

    #[inline]
    fn is_word_char(&self, c: i32) -> bool {
        c == '_' as i32 || self.isctype(c, Class::Alnum)
    }

    /// True when `assertions` fail at the cursor.
    // R: tre-match-utils.h:CHECK_ASSERTIONS
    pub(super) fn assertions_fail(&self, a: i32, c: &Cursor) -> bool {
        let nl = '\n' as i32;
        (a & ASSERT_AT_BOL != 0 && (c.pos > 0 || self.notbol) && (c.prev_c != nl || !self.newline))
            || (a & ASSERT_AT_EOL != 0 && (c.next_c != 0 || self.noteol) && (c.next_c != nl || !self.newline))
            || (a & ASSERT_AT_BOW != 0 && (self.is_word_char(c.prev_c) || !self.is_word_char(c.next_c)))
            || (a & ASSERT_AT_EOW != 0 && (!self.is_word_char(c.prev_c) || self.is_word_char(c.next_c)))
            || (a & ASSERT_AT_WB != 0
                && (c.pos != 0 && c.next_c != 0 && self.is_word_char(c.prev_c) == self.is_word_char(c.next_c)))
            || (a & ASSERT_AT_WB_NEG != 0
                && (c.pos == 0 || c.next_c == 0 || self.is_word_char(c.prev_c) != self.is_word_char(c.next_c)))
    }

    /// True when `prev_c` is outside the transition's classes.
    // R: tre-match-utils.h:CHECK_CHAR_CLASSES
    pub(super) fn classes_fail(&self, t: &Transition, c: i32) -> bool {
        if t.assertions & ASSERT_CHAR_CLASS != 0 {
            let class = t.class.expect("ASSERT_CHAR_CLASS without a class");
            let fails = if self.icase {
                !self.isctype(tolower(c as u32, self.locale) as i32, class)
                    && !self.isctype(toupper(c as u32, self.locale) as i32, class)
            } else {
                !self.isctype(c, class)
            };
            if fails {
                return true;
            }
        }
        t.assertions & ASSERT_CHAR_CLASS_NEG != 0 && self.neg_classes_match(t, c)
    }

    // R: tre-match-utils.h:tre_neg_char_classes_match
    fn neg_classes_match(&self, t: &Transition, c: i32) -> bool {
        let classes = t.neg_classes.as_deref().unwrap_or(&[]);
        classes.iter().any(|&k| {
            if self.icase {
                self.isctype(toupper(c as u32, self.locale) as i32, k)
                    || self.isctype(tolower(c as u32, self.locale) as i32, k)
            } else {
                self.isctype(c, k)
            }
        })
    }

    /// The transition reads `c` and its assertions hold.
    #[inline]
    fn takes(&self, t: &Transition, cur: &Cursor) -> bool {
        // `tre_cint_t` is unsigned.
        let c = cur.prev_c as u32;
        (t.code_min as u32) <= c
            && (t.code_max as u32) >= c
            && (t.assertions == 0 || !(self.assertions_fail(t.assertions, cur) || self.classes_fail(t, cur.prev_c)))
    }
}

/// True if the tag values `t1` win over `t2`.
// R: tre-match-utils.h:tre_tag_order
pub(super) fn tag_order(num_tags: usize, dirs: &[i32], t1: &[i32], t2: &[i32]) -> bool {
    for i in 0..num_tags {
        let minimize = dirs.get(i).copied().unwrap_or(-1) == TAG_MINIMIZE;
        if t1[i] != t2[i] {
            return (t1[i] < t2[i]) == minimize;
        }
    }
    false
}

/// A `regmatch_t`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Match {
    pub so: i32,
    pub eo: i32,
}

impl Match {
    pub const UNSET: Match = Match { so: -1, eo: -1 };
}

/// Fills `pmatch` from the tag values of a match ending at `match_eo`.
// R: regexec.c:tre_fill_pmatch
pub(super) fn fill_pmatch(
    pmatch: &mut [Match],
    cflags: i32,
    tnfa: &Tnfa,
    tags: &[i32],
    match_eo: i32,
) -> Result<(), Error> {
    let nmatch = pmatch.len();
    let mut i = 0;
    if match_eo >= 0 && cflags & REG_NOSUB == 0 {
        // Submatch offsets from the tags.
        let sd = &tnfa.submatch_data;
        while i < tnfa.num_submatches && i < nmatch {
            let tag = |t: i32| if t == tnfa.end_tag { match_eo } else { tags[t as usize] };
            let mut m = Match { so: tag(sd[i].so_tag), eo: tag(sd[i].eo_tag) };
            // An endpoint not used: this submatch was not part of the match.
            if m.so == -1 || m.eo == -1 {
                m = Match::UNSET;
            }
            pmatch[i] = m;
            i += 1;
        }
        // Reset the submatches not within all of their parents.
        i = 0;
        while i < tnfa.num_submatches && i < nmatch {
            if pmatch[i].eo == -1 && pmatch[i].so != -1 {
                return Err(Error::Assert { expr: "pmatch[i].rm_so == -1", file: "regexec.c", line: 101 });
            }
            if pmatch[i].so > pmatch[i].eo {
                return Err(Error::Assert { expr: "pmatch[i].rm_so <= pmatch[i].rm_eo", file: "regexec.c", line: 102 });
            }
            if let Some(parents) = &sd[i].parents {
                for &p in parents {
                    let p = pmatch[p as usize];
                    if pmatch[i].so < p.so || pmatch[i].eo > p.eo {
                        pmatch[i] = Match::UNSET;
                    }
                }
            }
            i += 1;
        }
    }
    for m in &mut pmatch[i..] {
        *m = Match::UNSET;
    }
    Ok(())
}

/// An entry of `reach`: a state and the tag buffer its path carries.
#[derive(Clone, Copy, Default)]
struct Reach {
    state: usize,
    tags: usize,
}

/// Runs the TNFA over `s` with all paths in parallel; the leftmost
/// match's end, or -1. `match_tags` is None when the caller wants no
/// submatches (C's NULL).
// R: tre-match-parallel.c:tre_tnfa_run_parallel
fn run_parallel<T: Text + ?Sized>(ctx: &Ctx, s: &T, len: i32, match_tags: Option<&mut [i32]>) -> Result<i32, Error> {
    let tnfa = ctx.tnfa;
    let mut no_tags: [i32; 0] = [];
    let (mt, nt) = match match_tags {
        Some(m) => (m, tnfa.num_tags as usize),
        None => (&mut no_tags[..], 0),
    };
    let n = tnfa.num_states as usize;
    // Tag buffers: `reach` entries own 0..n, `reach_next` n..2n, and
    // `tmp_tags` 2n; TRE swaps these pointers rather than copying.
    let mut tagmem = vec![0i32; (2 * n + 1) * nt];
    let mut reach: Vec<Reach> = (0..n).map(|i| Reach { state: 0, tags: i }).collect();
    let mut reach_next: Vec<Reach> = (0..n).map(|i| Reach { state: 0, tags: n + i }).collect();
    let mut tmp_tags = 2 * n;
    // `reach_pos[id]`: the position a state was last reached at and the
    // `reach_next` entry holding its tags.
    let mut reach_pos: Vec<(i32, usize)> = vec![(-1, 0); n];
    let mut match_eo = -1;
    let mut new_match = false;
    let dirs = &tnfa.tag_directions[..];
    let trans = &tnfa.transitions[..];

    let mut cur = Cursor::new(-1);
    // If only one character can start a match, find it first.
    if tnfa.first_char >= 0 && T::BYTES {
        let Some(p) = s.find(tnfa.first_char, len) else {
            return Ok(-1);
        };
        if p >= 1 {
            cur.prev_c = s.get(p as isize - 1);
        }
        cur.next_c = s.get(p as isize);
        cur.pos = p as i32;
        cur.idx = if len < 0 || cur.pos < len { p as isize + 1 } else { p as isize };
    } else {
        if !cur.next(s, len) {
            return Ok(-1);
        }
        cur.pos = 0;
    }

    let mut rn = 0; // reach_next_i
    loop {
        if match_eo < 0 {
            // No match yet: add the initial states to `reach_next`.
            for t in &tnfa.initial {
                let id = t.state_id as usize;
                if reach_pos[id].0 < cur.pos {
                    if t.assertions != 0 && ctx.assertions_fail(t.assertions, &cur) {
                        continue;
                    }
                    let state = t.state.expect("initial transition without a state");
                    reach_next[rn].state = state;
                    let b = reach_next[rn].tags * nt;
                    let buf = &mut tagmem[b..b + nt];
                    buf.fill(-1);
                    for &tg in &t.tags {
                        if (tg as usize) < nt {
                            buf[tg as usize] = cur.pos;
                        }
                    }
                    if state == tnfa.final_state {
                        match_eo = cur.pos;
                        new_match = true;
                        mt.copy_from_slice(buf);
                    }
                    reach_pos[id] = (cur.pos, rn);
                    rn += 1;
                }
            }
        } else if nt == 0 || rn == 0 {
            // We have found a match.
            break;
        }

        if cur.at_end(len) {
            break;
        }
        if !cur.next(s, len) {
            return Ok(-1);
        }

        std::mem::swap(&mut reach, &mut reach_next);
        let mut nr = rn;

        // Weed out states that don't fulfill the minimal matching
        // conditions.
        if tnfa.num_minimals != 0 && new_match {
            new_match = false;
            rn = 0;
            for i in 0..nr {
                let b = reach[i].tags * nt;
                let skip = tnfa.minimal_tags.iter().any(|&(end, start)| {
                    end as usize >= nt
                        || (tagmem[b + start as usize] == mt[start as usize]
                            && tagmem[b + end as usize] < mt[end as usize])
                });
                if !skip {
                    reach_next[rn].state = reach[i].state;
                    std::mem::swap(&mut reach_next[rn].tags, &mut reach[i].tags);
                    rn += 1;
                }
            }
            std::mem::swap(&mut reach, &mut reach_next);
            nr = rn;
        }

        // Follow the transitions on the current symbol into states not
        // yet in `reach_next`.
        rn = 0;
        for i in 0..nr {
            let (from_state, from_tags) = (reach[i].state, reach[i].tags);
            let mut ti = from_state;
            while trans[ti].state.is_some() {
                let t = &trans[ti];
                ti += 1;
                if !ctx.takes(t, &cur) {
                    continue;
                }
                // The tags after this transition.
                let (tb, fb) = (tmp_tags * nt, from_tags * nt);
                tagmem.copy_within(fb..fb + nt, tb);
                for &tg in &t.tags {
                    if (tg as usize) < nt {
                        tagmem[tb + tg as usize] = cur.pos;
                    }
                }
                let id = t.state_id as usize;
                let dest = t.state.unwrap();
                if reach_pos[id].0 < cur.pos {
                    // An unvisited state.
                    reach_next[rn].state = dest;
                    std::mem::swap(&mut reach_next[rn].tags, &mut tmp_tags);
                    reach_pos[id] = (cur.pos, rn);
                    let b = reach_next[rn].tags * nt;
                    if dest == tnfa.final_state && (match_eo == -1 || (nt > 0 && tagmem[b] <= mt[0])) {
                        match_eo = cur.pos;
                        new_match = true;
                        mt.copy_from_slice(&tagmem[b..b + nt]);
                    }
                    rn += 1;
                } else {
                    if reach_pos[id].0 != cur.pos {
                        return Err(Error::Assert {
                            expr: "reach_pos[trans_i->state_id].pos == pos",
                            file: "tre-match-parallel.c",
                            line: 462,
                        });
                    }
                    // Another path reached this state too: the tags pick
                    // the winner.
                    let e = reach_pos[id].1;
                    let ob = reach_next[e].tags * nt;
                    if tag_order(nt, dirs, &tagmem[tb..tb + nt], &tagmem[ob..ob + nt]) {
                        // The new path wins.
                        std::mem::swap(&mut reach_next[e].tags, &mut tmp_tags);
                        if dest == tnfa.final_state {
                            match_eo = cur.pos;
                            new_match = true;
                            mt.copy_from_slice(&tagmem[tb..tb + nt]);
                        }
                    }
                }
            }
        }
    }
    Ok(match_eo)
}

/// A saved alternative of the backtracking matcher.
struct BtItem {
    pos: i32,
    idx: isize,
    state: usize,
    state_id: i32,
    next_c: i32,
    tags: Vec<i32>,
}

/// Explores the TNFA's paths one at a time, for regexes with back
/// references; the best match's end, or -1.
// R: tre-match-backtrack.c:tre_tnfa_run_backtrack
fn run_backtrack<T: Text + ?Sized>(
    ctx: &Ctx,
    s: &T,
    len: i32,
    mut match_tags: Option<&mut [i32]>,
) -> Result<i32, Error> {
    // C's stack pop does not restore `pos_add_next`, and back references
    // step through the subject by characters, so on a multibyte string
    // positions would come out wrong; R never gets here (only literal
    // patterns are matched as STR_MBS).
    if T::MBS {
        return Err(Error::Unsupported("TRE's backtracking matcher on a multibyte string"));
    }
    let tnfa = ctx.tnfa;
    let trans = &tnfa.transitions[..];
    let nt = tnfa.num_tags as usize;
    let dirs = &tnfa.tag_directions[..];
    let mut tags = vec![-1i32; nt];
    let mut pmatch = vec![Match::UNSET; tnfa.num_submatches];
    let mut states_seen = vec![false; tnfa.num_states as usize];
    let mut stack: Vec<BtItem> = Vec::new();
    let mut match_eo = -1;

    let mut cur = Cursor::new(0);
    // Where the current search started, to retry from the next position.
    let mut pos_start = -1;

    let push = |stack: &mut Vec<BtItem>, cur: &Cursor, t: &Transition, tags: &[i32]| {
        let mut item_tags = tags.to_vec();
        for &tg in &t.tags {
            item_tags[tg as usize] = cur.pos;
        }
        stack.push(BtItem {
            pos: cur.pos,
            idx: cur.idx,
            state: t.state.unwrap(),
            state_id: t.state_id,
            next_c: cur.next_c,
            tags: item_tags,
        });
    };

    'retry: loop {
        tags.fill(-1);
        if let Some(m) = match_tags.as_deref_mut() {
            m.fill(-1);
        }
        states_seen.fill(false);
        cur.pos = pos_start;
        if !cur.next(s, len) {
            return Ok(-1);
        }
        pos_start = cur.pos;
        let (next_c_start, idx_start) = (cur.next_c, cur.idx);

        let mut state: Option<usize> = None;
        let mut next_tags: &[i32] = &[];
        for t in &tnfa.initial {
            if t.assertions != 0 && ctx.assertions_fail(t.assertions, &cur) {
                continue;
            }
            if state.is_none() {
                state = t.state;
                next_tags = &t.tags;
            } else {
                push(&mut stack, &cur, t, &tags);
            }
        }
        for &tg in next_tags {
            tags[tg as usize] = cur.pos;
        }

        // `state` is None only when no initial transition applied.
        'path: loop {
            'step: {
                let Some(st) = state else { break 'step };
                if st == tnfa.final_state {
                    let better = match_eo < cur.pos
                        || (match_eo == cur.pos
                            && match_tags.as_deref().is_some_and(|m| tag_order(nt, dirs, &tags, m)));
                    if better {
                        match_eo = cur.pos;
                        if let Some(m) = match_tags.as_deref_mut() {
                            m.copy_from_slice(&tags);
                        }
                    }
                    // No transitions leave the final state.
                    break 'step;
                }
                let t0 = &trans[st];
                if t0.state.is_some() && t0.assertions & ASSERT_BACKREF != 0 {
                    // A back reference state: match the referenced
                    // substring instead of reading a character.
                    let bt = t0.backref as usize;
                    fill_pmatch(&mut pmatch[..bt + 1], tnfa.cflags & !REG_NOSUB, tnfa, &tags, cur.pos)?;
                    let (so, eo) = (pmatch[bt].so, pmatch[bt].eo);
                    let bt_len = eo - so;
                    let equal = if len < 0 {
                        ncmp_eq(s, so as isize, cur.idx - 1, bt_len, true)
                    } else if len - cur.pos < bt_len {
                        false
                    } else {
                        ncmp_eq(s, so as isize, cur.idx - 1, bt_len, false)
                    };
                    if !equal {
                        break 'step;
                    }
                    let empty_br_match = bt_len == 0;
                    let id = t0.state_id as usize;
                    if empty_br_match && states_seen[id] {
                        break 'step;
                    }
                    states_seen[id] = empty_br_match;
                    // Advance and resync (`prev_c` stays stale).
                    cur.idx += bt_len as isize - 1;
                    cur.pos += bt_len - 1;
                    if !cur.next(s, len) {
                        return Ok(-1);
                    }
                } else if cur.at_end(len) {
                    break 'step;
                } else {
                    if !cur.next(s, len) {
                        return Ok(-1);
                    }
                }

                let mut next_state: Option<usize> = None;
                let mut ti = st;
                while trans[ti].state.is_some() {
                    let t = &trans[ti];
                    ti += 1;
                    if !ctx.takes(t, &cur) {
                        continue;
                    }
                    if next_state.is_none() {
                        next_state = t.state;
                        next_tags = &t.tags;
                    } else {
                        // A second way on: remember it.
                        push(&mut stack, &cur, t, &tags);
                    }
                }
                if let Some(ns) = next_state {
                    state = Some(ns);
                    for &tg in next_tags {
                        tags[tg as usize] = cur.pos;
                    }
                    continue 'path;
                }
            }

            // No transition matched: backtrack.
            if let Some(item) = stack.pop() {
                if trans[item.state].assertions & ASSERT_BACKREF != 0 {
                    states_seen[item.state_id as usize] = false;
                }
                cur.pos = item.pos;
                cur.idx = item.idx;
                state = Some(item.state);
                cur.next_c = item.next_c;
                tags = item.tags;
            } else if match_eo < 0 {
                // Try starting from a later position.
                if cur.at_end(len) {
                    break 'retry;
                }
                cur.next_c = next_c_start;
                cur.idx = idx_start;
                continue 'retry;
            } else {
                break 'retry;
            }
        }
    }
    Ok(match_eo)
}

/// Matches `s` (of length `len`, or up to its end when `len < 0`) and
/// fills `pmatch`. `Ok(false)` is REG_NOMATCH.
// R: regexec.c:tre_match
pub(super) fn tre_match<T: Text + ?Sized>(
    tnfa: &Tnfa,
    locale: Locale,
    s: &T,
    len: i32,
    pmatch: &mut [Match],
    eflags: i32,
) -> Result<bool, Error> {
    let ctx = Ctx::new(tnfa, eflags, locale);
    let mut tags =
        if tnfa.num_tags > 0 && !pmatch.is_empty() { Some(vec![0i32; tnfa.num_tags as usize]) } else { None };
    let eo = if tnfa.have_backrefs || eflags & super::REG_BACKTRACKING_MATCHER != 0 {
        run_backtrack(&ctx, s, len, tags.as_deref_mut())?
    } else if tnfa.have_approx || eflags & super::REG_APPROX_MATCHER != 0 {
        // Edits only where the regex's parameters allow them.
        let params = AParams { max_err: 0, max_cost: 0, ..AParams::default() };
        run_approx(&ctx, s, len, tags.as_deref_mut(), &params)?.0
    } else {
        run_parallel(&ctx, s, len, tags.as_deref_mut())?
    };
    if eo < 0 {
        return Ok(false);
    }
    fill_pmatch(pmatch, tnfa.cflags, tnfa, tags.as_deref().unwrap_or(&[]), eo)?;
    Ok(true)
}

/// Matches `s` approximately within `params` and fills `pmatch`; the
/// match's costs, or `None` for REG_NOMATCH.
// R: regexec.c:tre_match_approx
pub(super) fn tre_match_approx<T: Text + ?Sized>(
    tnfa: &Tnfa,
    locale: Locale,
    s: &T,
    len: i32,
    pmatch: &mut [Match],
    params: &AParams,
    eflags: i32,
) -> Result<Option<AMatch>, Error> {
    // Without approximate features, a maximum cost of zero or a forced
    // approximate matcher, the exact matcher does.
    if params.max_cost == 0 && !tnfa.have_approx && eflags & super::REG_APPROX_MATCHER == 0 {
        return Ok(tre_match(tnfa, locale, s, len, pmatch, eflags)?.then(AMatch::default));
    }
    // The approximate matcher does not support back references.
    if tnfa.have_backrefs {
        return Err(Error::Status(super::Status::Badpat));
    }
    let ctx = Ctx::new(tnfa, eflags, locale);
    let mut tags =
        if tnfa.num_tags > 0 && !pmatch.is_empty() { Some(vec![0i32; tnfa.num_tags as usize]) } else { None };
    let (eo, m) = run_approx(&ctx, s, len, tags.as_deref_mut(), params)?;
    if eo < 0 {
        return Ok(None);
    }
    fill_pmatch(pmatch, tnfa.cflags, tnfa, tags.as_deref().unwrap_or(&[]), eo)?;
    Ok(Some(m))
}
