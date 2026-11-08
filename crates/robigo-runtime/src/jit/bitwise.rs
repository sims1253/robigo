//! Scalar lowering of R's bitwise internals (R: relop.c:bitwiseAnd et al.).
use super::*;

#[derive(Clone, Copy)]
pub(super) enum BitOp {
    And,
    Or,
    Xor,
    Not,
    ShiftL,
    ShiftR,
}

pub(super) fn call(code: &Code, op: &Op) -> Option<(BitOp, Vec<Reg>)> {
    let Op::CallBuiltin { f, args, .. } = op else { return None };
    let Value::Builtin(b) = &code.consts[*f as usize] else { return None };
    let op = match b.name {
        "bitwiseAnd" => BitOp::And,
        "bitwiseOr" => BitOp::Or,
        "bitwiseXor" => BitOp::Xor,
        "bitwiseNot" => BitOp::Not,
        "bitwiseShiftL" => BitOp::ShiftL,
        "bitwiseShiftR" => BitOp::ShiftR,
        _ => return None,
    };
    let args = &code.arglists[*args as usize];
    if args.len() != if matches!(op, BitOp::Not) { 1 } else { 2 } {
        return None;
    }
    Some((op, args.iter().map(|(_, r)| *r).collect()))
}

impl Gen<'_, '_> {
    pub(super) fn bitwise(&mut self, op: BitOp, regs: &[Reg], dst: Reg, pc: usize) {
        let mut xs = Vec::new();
        for r in regs {
            let (x, t) = self.reg(*r);
            let x = if matches!(t, JType::D | JType::N) {
                // R: coerce.c:IntegerFromReal. Replay warning-producing
                // conversions and NaNs in the VM, before writing any result.
                let lo = self.f64c(i32::MIN as f64);
                let hi = self.f64c(i32::MAX as f64 + 1.0);
                let a = self.b.ins().fcmp(FloatCC::GreaterThan, x, lo);
                let b = self.b.ins().fcmp(FloatCC::LessThan, x, hi);
                let ok = self.b.ins().band(a, b);
                let bad = self.b.ins().bxor_imm_s(ok, 1);
                self.deopt_if(bad, pc);
                self.b.ins().fcvt_to_sint(types::I32, x)
            } else {
                x
            };
            xs.push(x);
        }
        let a = xs[0];
        let mut na = self.is_na_i(a);
        let result = if matches!(op, BitOp::Not) {
            self.b.ins().bnot(a)
        } else {
            let b = xs[1];
            let nb = self.is_na_i(b);
            na = self.b.ins().bor(na, nb);
            match op {
                BitOp::And => self.b.ins().band(a, b),
                BitOp::Or => self.b.ins().bor(a, b),
                BitOp::Xor => self.b.ins().bxor(a, b),
                BitOp::ShiftL | BitOp::ShiftR => {
                    let bad = self.b.ins().icmp_imm_u(IntCC::UnsignedGreaterThan, b, 31);
                    na = self.b.ins().bor(na, bad);
                    if matches!(op, BitOp::ShiftL) { self.b.ins().ishl(a, b) } else { self.b.ins().ushr(a, b) }
                }
                BitOp::Not => unreachable!(),
            }
        };
        let missing = self.i32c(NA_INTEGER as i64);
        let result = self.b.ins().select(na, missing, result);
        self.set_reg(dst, result);
    }
}

/// These bodies observe an integer or double argument only through integer
/// coercion, so a tagged number may be passed as its double value.
pub(super) fn numeric_args(code: &Code) -> bool {
    let mut result = None;
    for op in &code.ops {
        match (op, result) {
            (Op::GetVar { .. } | Op::Guard { .. }, None) | (Op::Visible { .. }, _) => {}
            (Op::CallBuiltin { dst, .. }, None) if call(code, op).is_some() => result = Some(*dst),
            (Op::Return { src }, Some(dst)) if *src == dst => {}
            _ => return false,
        }
    }
    result == Some(code.result)
}
