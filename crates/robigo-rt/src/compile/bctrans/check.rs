//! The byte code's operands checked before it is translated (#976, layer
//! 1): what bcEval and the translation read each operand as, by opcode
//! (GNU R 4.6.2's eval.c): a constant index of a kind, a label, or a plain
//! count. GNU R checks none of this (its compiler never makes code that
//! breaks it, and bcEval reads code it is handed as it is), so code made
//! another way (`.Internal(mkCode)`, a crafted serialization or lazy-load
//! database) can crash it. robigo-rt refuses it instead: the translation
//! stops with [`Need::Malformed`], and the code raises "robigo: malformed
//! byte code: ..." when it runs.
//!
//! After [`check`], the translation's reads of the constants (`k(i.a[n])`)
//! are within the pool, and each constant is of the kind its operand
//! names: a call is a proper list (a `LANGSXP` whose `CDR`s are
//! `LISTSXP`s ending in `NULL`), so walking its arguments ends.

use super::BcIns;
use super::opc::*;
use robigo_heap::Sexp;
use robigo_heap::types::*;

/// What an operand is.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum K {
    /// A constant of any kind.
    Any,
    /// A symbol.
    Sym,
    /// A call (a proper list).
    Call,
    /// An argument's tag: a symbol or a string (`CreateTag`'s).
    Tag,
    /// A `switch`'s names: a character vector or NULL.
    Names,
    /// A `switch`'s jump offsets: an integer vector or NULL.
    Offsets,
    /// `MAKECLOSURE`'s list: the formals (a pairlist or NULL), the body,
    /// and the srcref if a third.
    Closure,
    /// A code position.
    Label,
    /// A count, not negative.
    Count,
    /// A plain integer (a flag or an index the translation checks).
    Int,
}

use K::*;

/// The operands of opcode `op`, as many as it has.
const fn operands(op: u8) -> &'static [K] {
    match op {
        GOTO | STEPFOR => &[Label],
        BRIFNOT | STARTSUBSET | STARTSUBASSIGN | STARTC | STARTSUBSET2 | STARTSUBASSIGN2 | AND1ST | OR1ST
        | STARTSUBSET_N | STARTSUBASSIGN_N | STARTSUBSET2_N | STARTSUBASSIGN2_N | BASEGUARD => &[Call, Label],
        STARTLOOPCNTXT => &[Int, Label],
        ENDLOOPCNTXT => &[Int],
        STARTFOR => &[Call, Sym, Label],
        LDCONST | PUSHCONSTARG | MAKEPROM => &[Any],
        GETVAR | DDVAL | SETVAR | GETFUN | GETGLOBFUN | GETSYMFUN | GETBUILTIN | GETINTLBUILTIN | STARTASSIGN
        | ENDASSIGN | GETVAR_MISSOK | DDVAL_MISSOK | SETVAR2 | STARTASSIGN2 | ENDASSIGN2 => &[Sym],
        SETTAG => &[Tag],
        CALL | CALLBUILTIN | CALLSPECIAL | UMINUS | UPLUS | ADD | SUB | MUL | DIV | EXPT | SQRT | EXP | EQ | NE
        | LT | LE | GE | GT | AND | OR | NOT | VECSUBSET | MATSUBSET | VECSUBASSIGN | MATSUBASSIGN | AND2ND
        | OR2ND | GETTER_CALL | VECSUBSET2 | MATSUBSET2 | VECSUBASSIGN2 | MATSUBASSIGN2 | LOG | LOGBASE | COLON
        | SEQALONG | SEQLEN => &[Call],
        MAKECLOSURE => &[Closure],
        DOLLAR | DOLLARGETS => &[Call, Sym],
        SETTER_CALL => &[Call, Any],
        SWITCH => &[Call, Names, Offsets, Offsets],
        SUBSET_N | SUBSET2_N | SUBASSIGN_N | SUBASSIGN2_N | DOTCALL => &[Call, Count],
        MATH1 => &[Call, Int],
        DECLNK_N => &[Count],
        _ => &[],
    }
}

/// [`operands`] by opcode, made when this is compiled: the check reads a
/// table, not a match, per instruction.
const OPERANDS: [&[K]; 256] = {
    let mut t: [&[K]; 256] = [&[]; 256];
    let mut op = 0;
    while op < 256 {
        t[op] = operands(op as u8);
        op += 1;
    }
    t
};

/// The longest argument list a call constant may have: the matcher's
/// limit (`args.rs`'s `NO`), past which a walk of a cyclic list stops.
const MAX_ARGS: usize = u16::MAX as usize;

/// Whether `x` is a call whose arguments are a proper list.
///
/// # Safety
/// `x` is a live object.
unsafe fn proper_call(x: Sexp) -> bool {
    // SAFETY: the caller's contract; each cell read is a LISTSXP's.
    unsafe {
        if x.typ() != LANGSXP {
            return false;
        }
        let mut c = x.cdr();
        for _ in 0..=MAX_ARGS {
            match c.typ() {
                NILSXP => return true,
                LISTSXP => c = c.cdr(),
                _ => return false,
            }
        }
        false
    }
}

/// Whether `x` is a pairlist (a proper one) or NULL.
///
/// # Safety
/// `x` is a live object.
unsafe fn proper_pairlist(x: Sexp) -> bool {
    // SAFETY: as [`proper_call`]'s.
    unsafe {
        let mut c = x;
        for _ in 0..=MAX_ARGS {
            match c.typ() {
                NILSXP => return true,
                LISTSXP => c = c.cdr(),
                _ => return false,
            }
        }
        false
    }
}

/// Why constant `x` is not of kind `k`, or None if it is.
///
/// # Safety
/// `x` is a live object.
unsafe fn misfit(x: Sexp, k: K) -> Option<&'static str> {
    // SAFETY: the caller's contract; vector reads within their lengths.
    unsafe {
        let ok = match k {
            Any => true,
            Sym => x.typ() == SYMSXP,
            Call => proper_call(x),
            Tag => x.typ() == SYMSXP || (x.typ() == STRSXP && x.std_len() == 1),
            Names => matches!(x.typ(), STRSXP | NILSXP),
            Offsets => matches!(x.typ(), INTSXP | NILSXP),
            Closure => {
                x.typ() == VECSXP && x.std_len() >= 2 && {
                    let p = x.std_data() as *const Sexp;
                    proper_pairlist(*p)
                }
            }
            Label | Count | Int => true,
        };
        (!ok).then_some(match k {
            Sym => "a symbol",
            Call => "a call",
            Tag => "a symbol or a string",
            Names => "a character vector or NULL",
            Offsets => "an integer vector or NULL",
            Closure => "a closure's formals and body",
            _ => "",
        })
    }
}

/// Check `dec`'s operands against the constants `consts`: why the code is
/// malformed, or Ok. The labels are checked to be positions; whether one
/// starts an instruction is the translation's index's to say.
///
/// # Safety
/// `consts` is a live object (a BCODESXP's constants).
pub(super) unsafe fn check(dec: &[BcIns], consts: Sexp, code_len: usize) -> Result<(), String> {
    // SAFETY: the caller's contract; the pool's elements are read within
    // its length.
    unsafe {
        // (any constants, mkCode's: code whose operands read none runs with
        // them as in GNU R, its NULL included; a constant operand needs a
        // list to index)
        let list = consts.typ() == VECSXP;
        let n = if list { consts.std_len() as usize } else { 0 };
        let pool = if list { consts.std_data() as *const Sexp } else { std::ptr::null() };
        let name = |op: u8| robigo_rtbuiltins::bytecode::OPCODES.get(op as usize).map_or("?", |o| o.0);
        for i in dec {
            for (j, &k) in OPERANDS[i.op as usize].iter().enumerate() {
                let v = i.a[j];
                match k {
                    Label if v < 0 || v as usize > code_len => {
                        return Err(format!("{}'s label {v} is outside the code", name(i.op)));
                    }
                    // cmp.R's labels go forward but for GOTO's (a loop's
                    // back edge, which the translation polls at) and
                    // STEPFOR's (the body's start): a jump back from any
                    // other would loop where no poll is
                    Label if (v as usize) <= i.at && !matches!(i.op, GOTO | STEPFOR) => {
                        return Err(format!("{}'s label {v} is not after it", name(i.op)));
                    }
                    Count if v < 0 => return Err(format!("{}'s count {v} is negative", name(i.op))),
                    Label | Count | Int => {}
                    _ if !list => {
                        return Err(format!("{}'s constant {v} is read from constants that are not a list", name(i.op)));
                    }
                    _ if v < 0 || v as usize >= n => {
                        return Err(format!("{}'s constant {v} is outside its {n} constants", name(i.op)));
                    }
                    _ => {
                        let x = *pool.add(v as usize);
                        if let Some(want) = misfit(x, k) {
                            return Err(format!("{}'s constant {v} is not {want}", name(i.op)));
                        }
                        // a switch's jumps go forward, as cmp.R's do
                        if k == Offsets && x.typ() == INTSXP {
                            let o = std::slice::from_raw_parts(x.std_data() as *const i32, x.std_len() as usize);
                            if o.iter().any(|&p| p < 0 || p as usize <= i.at || p as usize > code_len) {
                                return Err(format!("{}'s jump table {v} jumps back or out of the code", name(i.op)));
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_opcode_with_operands_has_their_kinds() {
        // the table names as many operands as the opcode has (OPCODES'
        // argc), for every opcode that has any
        for (op, &(name, argc)) in robigo_rtbuiltins::bytecode::OPCODES.iter().enumerate() {
            let kinds = operands(op as u8);
            assert!(kinds.len() == argc as usize, "{name}: {} kinds for {argc} operands", kinds.len());
        }
    }
}
