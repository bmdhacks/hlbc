//! Def/Use extraction from opcodes
//!
//! For each opcode, we determine:
//! - Which registers it DEFINES (writes to)
//! - Which registers it USES (reads from)

use hlbc::opcodes::Opcode;
use hlbc::types::Reg;

/// Get the registers defined (written) by an opcode.
pub fn get_defs(op: &Opcode) -> Vec<Reg> {
    use Opcode::*;
    match op {
        // dst = ...
        Mov { dst, .. }
        | Int { dst, .. }
        | Float { dst, .. }
        | Bool { dst, .. }
        | Bytes { dst, .. }
        | String { dst, .. }
        | Null { dst }
        | Add { dst, .. }
        | Sub { dst, .. }
        | Mul { dst, .. }
        | SDiv { dst, .. }
        | UDiv { dst, .. }
        | SMod { dst, .. }
        | UMod { dst, .. }
        | Shl { dst, .. }
        | SShr { dst, .. }
        | UShr { dst, .. }
        | And { dst, .. }
        | Or { dst, .. }
        | Xor { dst, .. }
        | Neg { dst, .. }
        | Not { dst, .. }
        | Call0 { dst, .. }
        | Call1 { dst, .. }
        | Call2 { dst, .. }
        | Call3 { dst, .. }
        | Call4 { dst, .. }
        | CallN { dst, .. }
        | CallMethod { dst, .. }
        | CallThis { dst, .. }
        | CallClosure { dst, .. }
        | StaticClosure { dst, .. }
        | InstanceClosure { dst, .. }
        | VirtualClosure { dst, .. }
        | GetGlobal { dst, .. }
        | Field { dst, .. }
        | GetThis { dst, .. }
        | DynGet { dst, .. }
        | ToDyn { dst, .. }
        | ToSFloat { dst, .. }
        | ToUFloat { dst, .. }
        | ToInt { dst, .. }
        | SafeCast { dst, .. }
        | UnsafeCast { dst, .. }
        | ToVirtual { dst, .. }
        | GetI8 { dst, .. }
        | GetI16 { dst, .. }
        | GetMem { dst, .. }
        | GetArray { dst, .. }
        | New { dst }
        | ArraySize { dst, .. }
        | Type { dst, .. }
        | GetType { dst, .. }
        | GetTID { dst, .. }
        | Ref { dst, .. }
        | Unref { dst, .. }
        | MakeEnum { dst, .. }
        | EnumAlloc { dst, .. }
        | EnumIndex { dst, .. }
        | EnumField { dst, .. }
        | RefData { dst, .. }
        | RefOffset { dst, .. } => vec![*dst],

        // Incr/Decr both read and write dst
        Incr { dst } | Decr { dst } => vec![*dst],

        // Setref writes through a reference, but the reference register itself is modified
        // Actually, Setref writes *through* dst, so dst is used not defined
        Setref { .. } => vec![],

        // Trap defines the exception register on the catch path
        Trap { exc, .. } => vec![*exc],

        // These don't define any registers
        SetGlobal { .. }
        | SetField { .. }
        | SetThis { .. }
        | DynSet { .. }
        | JTrue { .. }
        | JFalse { .. }
        | JNull { .. }
        | JNotNull { .. }
        | JSLt { .. }
        | JSGte { .. }
        | JSGt { .. }
        | JSLte { .. }
        | JULt { .. }
        | JUGte { .. }
        | JNotLt { .. }
        | JNotGte { .. }
        | JEq { .. }
        | JNotEq { .. }
        | JAlways { .. }
        | Label
        | Ret { .. }
        | Throw { .. }
        | Rethrow { .. }
        | Switch { .. }
        | NullCheck { .. }
        | EndTrap { .. }
        | SetI8 { .. }
        | SetI16 { .. }
        | SetMem { .. }
        | SetArray { .. }
        | SetEnumField { .. }
        | Assert
        | Nop
        | Prefetch { .. }
        | Asm { .. } => vec![],
    }
}

/// Get the registers used (read) by an opcode.
pub fn get_uses(op: &Opcode) -> Vec<Reg> {
    use Opcode::*;
    match op {
        // Simple moves/conversions read src
        Mov { src, .. }
        | ToDyn { src, .. }
        | ToSFloat { src, .. }
        | ToUFloat { src, .. }
        | ToInt { src, .. }
        | SafeCast { src, .. }
        | UnsafeCast { src, .. }
        | ToVirtual { src, .. }
        | Neg { src, .. }
        | Not { src, .. }
        | Ref { src, .. }
        | Unref { src, .. }
        | GetType { src, .. }
        | GetTID { src, .. }
        | RefData { src, .. } => vec![*src],

        // Binary ops read a and b
        Add { a, b, .. }
        | Sub { a, b, .. }
        | Mul { a, b, .. }
        | SDiv { a, b, .. }
        | UDiv { a, b, .. }
        | SMod { a, b, .. }
        | UMod { a, b, .. }
        | Shl { a, b, .. }
        | SShr { a, b, .. }
        | UShr { a, b, .. }
        | And { a, b, .. }
        | Or { a, b, .. }
        | Xor { a, b, .. } => vec![*a, *b],

        // Incr/Decr read and write dst
        Incr { dst } | Decr { dst } => vec![*dst],

        // Call instructions
        Call0 { .. } => vec![],
        Call1 { arg0, .. } => vec![*arg0],
        Call2 { arg0, arg1, .. } => vec![*arg0, *arg1],
        Call3 { arg0, arg1, arg2, .. } => vec![*arg0, *arg1, *arg2],
        Call4 {
            arg0,
            arg1,
            arg2,
            arg3,
            ..
        } => vec![*arg0, *arg1, *arg2, *arg3],
        CallN { args, .. } | CallMethod { args, .. } | CallThis { args, .. } => args.clone(),
        CallClosure { fun, args, .. } => {
            let mut uses = vec![*fun];
            uses.extend(args.iter().copied());
            uses
        }

        // Closures
        StaticClosure { .. } => vec![],
        InstanceClosure { obj, .. } => vec![*obj],
        VirtualClosure { obj, field, .. } => vec![*obj, *field],

        // Globals
        GetGlobal { .. } => vec![],
        SetGlobal { src, .. } => vec![*src],

        // Field access
        Field { obj, .. } => vec![*obj],
        SetField { obj, src, .. } => vec![*obj, *src],
        GetThis { .. } => vec![], // Implicit this = reg0
        SetThis { src, .. } => vec![*src],
        DynGet { obj, .. } => vec![*obj],
        DynSet { obj, src, .. } => vec![*obj, *src],

        // Jumps
        JTrue { cond, .. } | JFalse { cond, .. } => vec![*cond],
        JNull { reg, .. } | JNotNull { reg, .. } => vec![*reg],
        JSLt { a, b, .. }
        | JSGte { a, b, .. }
        | JSGt { a, b, .. }
        | JSLte { a, b, .. }
        | JULt { a, b, .. }
        | JUGte { a, b, .. }
        | JNotLt { a, b, .. }
        | JNotGte { a, b, .. }
        | JEq { a, b, .. }
        | JNotEq { a, b, .. } => vec![*a, *b],
        JAlways { .. } => vec![],

        // Control flow
        Label => vec![],
        Ret { ret } => vec![*ret],
        Throw { exc } | Rethrow { exc } => vec![*exc],
        Switch { reg, .. } => vec![*reg],
        NullCheck { reg } => vec![*reg],
        Trap { .. } => vec![],
        EndTrap { exc } => vec![*exc],

        // Memory/array access
        GetI8 { bytes, index, .. }
        | GetI16 { bytes, index, .. }
        | GetMem { bytes, index, .. } => vec![*bytes, *index],
        GetArray { array, index, .. } => vec![*array, *index],
        SetI8 { bytes, index, src }
        | SetI16 { bytes, index, src }
        | SetMem { bytes, index, src } => vec![*bytes, *index, *src],
        SetArray { array, index, src } => vec![*array, *index, *src],

        // Object allocation
        New { .. } => vec![],
        ArraySize { array, .. } => vec![*array],

        // Type introspection
        Type { .. } => vec![],

        // References
        Setref { dst, value } => vec![*dst, *value], // dst is used (written through), value is used

        // Enums
        MakeEnum { args, .. } => args.clone(),
        EnumAlloc { .. } => vec![],
        EnumIndex { value, .. } => vec![*value],
        EnumField { value, .. } => vec![*value],
        SetEnumField { value, src, .. } => vec![*value, *src],

        // Misc
        Assert | Nop => vec![],
        Int { .. } | Float { .. } | Bool { .. } | Bytes { .. } | String { .. } | Null { .. } => {
            vec![]
        }
        Prefetch { value, .. } => vec![*value],
        Asm { reg, .. } => {
            // reg is only valid if non-zero
            if reg.0 != 0 {
                vec![Reg(reg.0 - 1)]
            } else {
                vec![]
            }
        }
        RefOffset { reg, offset, .. } => vec![*reg, *offset],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hlbc::types::{RefField, RefFun, RefInt};

    #[test]
    fn test_mov_def_use() {
        let op = Opcode::Mov {
            dst: Reg(5),
            src: Reg(3),
        };
        assert_eq!(get_defs(&op), vec![Reg(5)]);
        assert_eq!(get_uses(&op), vec![Reg(3)]);
    }

    #[test]
    fn test_add_def_use() {
        let op = Opcode::Add {
            dst: Reg(5),
            a: Reg(2),
            b: Reg(3),
        };
        assert_eq!(get_defs(&op), vec![Reg(5)]);
        assert_eq!(get_uses(&op), vec![Reg(2), Reg(3)]);
    }

    #[test]
    fn test_call_def_use() {
        let op = Opcode::Call2 {
            dst: Reg(0),
            fun: RefFun(0),
            arg0: Reg(1),
            arg1: Reg(2),
        };
        assert_eq!(get_defs(&op), vec![Reg(0)]);
        assert_eq!(get_uses(&op), vec![Reg(1), Reg(2)]);
    }

    #[test]
    fn test_incr_is_both() {
        let op = Opcode::Incr { dst: Reg(5) };
        assert_eq!(get_defs(&op), vec![Reg(5)]);
        assert_eq!(get_uses(&op), vec![Reg(5)]);
    }
}
