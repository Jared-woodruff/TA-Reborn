//! COB (compiled BOS unit script) reader and disassembler.
//!
//! Each unit has a script that animates pieces and answers engine queries (aiming, build
//! positions, ...). Header (TA = version 4, 11 × i32): version, script count, piece count,
//! code length (in u32 words), static variable count, 0, script code-offset table ptr,
//! script name-ptr table ptr, piece name-ptr table ptr, code ptr, 0.

use crate::bin::{Budget, cstr_at, i32_at, u32_at, table};
use crate::{FormatError, Result};

#[derive(Clone, Debug)]
pub struct CobFunction {
    pub name: String,
    /// Entry point, as an index into [`CobScript::code`].
    pub offset: u32,
}

#[derive(Clone, Debug, Default)]
pub struct CobScript {
    pub functions: Vec<CobFunction>,
    pub pieces: Vec<String>,
    pub code: Vec<u32>,
    pub static_var_count: u32,
}

impl CobScript {
    pub fn parse(data: &[u8]) -> Result<CobScript> {
        let version = i32_at(data, 0)?;
        if version != 4 && version != 6 {
            return Err(FormatError::invalid("COB", format!("unsupported version {version}")));
        }
        let script_count = i32_at(data, 4)?.max(0) as usize;
        let piece_count = i32_at(data, 8)?.max(0) as usize;
        let code_len = i32_at(data, 12)?.max(0) as usize;
        let static_var_count = i32_at(data, 16)?.max(0) as u32;
        let offsets_ptr = i32_at(data, 24)? as usize;
        let names_ptr = i32_at(data, 28)? as usize;
        let pieces_ptr = i32_at(data, 32)? as usize;
        let code_ptr = i32_at(data, 36)? as usize;
        if script_count > data.len() / 4 || piece_count > data.len() / 4 || code_len > data.len() / 4 {
            return Err(FormatError::invalid("COB", "implausible header counts"));
        }
        if static_var_count > 4096 || script_count > 65536 || piece_count > 1024 {
            return Err(FormatError::invalid("COB", "script, piece or static-variable count exceeds resource limit"));
        }
        table(data, offsets_ptr, script_count, 4)?;
        table(data, names_ptr, script_count, 4)?;
        table(data, pieces_ptr, piece_count, 4)?;
        table(data, code_ptr, code_len, 4)?;
        let mut budget = Budget(64 << 20);
        budget.take(code_len * 4, "COB")?;

        let mut code = Vec::with_capacity(code_len);
        for i in 0..code_len {
            code.push(u32_at(data, code_ptr + i * 4)?);
        }
        let mut functions = Vec::with_capacity(script_count);
        for i in 0..script_count {
            let offset = u32_at(data, offsets_ptr + i * 4)?;
            if offset as usize >= code_len {
                return Err(FormatError::invalid("COB", "function entry point lies outside code"));
            }
            let name = cstr_at(data, i32_at(data, names_ptr + i * 4)? as usize)?;
            budget.take(name.len() + std::mem::size_of::<CobFunction>(), "COB")?;
            functions.push(CobFunction { name, offset });
        }
        let mut pieces = Vec::with_capacity(piece_count);
        for i in 0..piece_count {
            pieces.push(cstr_at(data, i32_at(data, pieces_ptr + i * 4)? as usize)?);
        }
        Ok(CobScript { functions, pieces, code, static_var_count })
    }

    /// Case-insensitive function lookup.
    pub fn function(&self, name: &str) -> Option<usize> {
        self.functions.iter().position(|f| f.name.eq_ignore_ascii_case(name))
    }
}

/// COB opcodes. Operand counts are given in [`operand_count`].
pub mod op {
    pub const MOVE: u32 = 0x1000_1000;
    pub const TURN: u32 = 0x1000_2000;
    pub const SPIN: u32 = 0x1000_3000;
    pub const STOP_SPIN: u32 = 0x1000_4000;
    pub const SHOW: u32 = 0x1000_5000;
    pub const HIDE: u32 = 0x1000_6000;
    pub const CACHE: u32 = 0x1000_7000;
    pub const DONT_CACHE: u32 = 0x1000_8000;
    pub const MOVE_NOW: u32 = 0x1000_B000;
    pub const TURN_NOW: u32 = 0x1000_C000;
    pub const SHADE: u32 = 0x1000_D000;
    pub const DONT_SHADE: u32 = 0x1000_E000;
    pub const EMIT_SFX: u32 = 0x1000_F000;
    pub const WAIT_TURN: u32 = 0x1001_1000;
    pub const WAIT_MOVE: u32 = 0x1001_2000;
    pub const SLEEP: u32 = 0x1001_3000;
    pub const PUSH_CONSTANT: u32 = 0x1002_1001;
    pub const PUSH_LOCAL: u32 = 0x1002_1002;
    pub const PUSH_STATIC: u32 = 0x1002_1004;
    pub const CREATE_LOCAL: u32 = 0x1002_2000;
    pub const POP_LOCAL: u32 = 0x1002_3002;
    pub const POP_STATIC: u32 = 0x1002_3004;
    pub const POP_STACK: u32 = 0x1002_4000;
    pub const ADD: u32 = 0x1003_1000;
    pub const SUB: u32 = 0x1003_2000;
    pub const MUL: u32 = 0x1003_3000;
    pub const DIV: u32 = 0x1003_4000;
    pub const BITWISE_AND: u32 = 0x1003_5000;
    pub const BITWISE_OR: u32 = 0x1003_6000;
    pub const BITWISE_XOR: u32 = 0x1003_7000;
    pub const BITWISE_NOT: u32 = 0x1003_8000;
    pub const RAND: u32 = 0x1004_1000;
    pub const GET_UNIT_VALUE: u32 = 0x1004_2000;
    pub const GET: u32 = 0x1004_3000;
    pub const SET_LESS: u32 = 0x1005_1000;
    pub const SET_LESS_OR_EQUAL: u32 = 0x1005_2000;
    pub const SET_GREATER: u32 = 0x1005_3000;
    pub const SET_GREATER_OR_EQUAL: u32 = 0x1005_4000;
    pub const SET_EQUAL: u32 = 0x1005_5000;
    pub const SET_NOT_EQUAL: u32 = 0x1005_6000;
    pub const LOGICAL_AND: u32 = 0x1005_7000;
    pub const LOGICAL_OR: u32 = 0x1005_8000;
    pub const LOGICAL_XOR: u32 = 0x1005_9000;
    pub const LOGICAL_NOT: u32 = 0x1005_A000;
    pub const START_SCRIPT: u32 = 0x1006_1000;
    pub const CALL_SCRIPT: u32 = 0x1006_2000;
    pub const JUMP: u32 = 0x1006_4000;
    pub const RETURN: u32 = 0x1006_5000;
    pub const JUMP_IF_FALSE: u32 = 0x1006_6000;
    pub const SIGNAL: u32 = 0x1006_7000;
    pub const SET_SIGNAL_MASK: u32 = 0x1006_8000;
    pub const EXPLODE: u32 = 0x1007_1000;
    pub const PLAY_SOUND: u32 = 0x1007_2000;
    pub const SET_VALUE: u32 = 0x1008_2000;
    pub const ATTACH_UNIT: u32 = 0x1008_3000;
    pub const DROP_UNIT: u32 = 0x1008_4000;
}

/// Number of inline operand words following an opcode, or `None` for unknown opcodes.
pub fn operand_count(opcode: u32) -> Option<usize> {
    use op::*;
    Some(match opcode {
        MOVE | TURN | SPIN | STOP_SPIN | WAIT_TURN | WAIT_MOVE | MOVE_NOW | TURN_NOW => 2,
        SHOW | HIDE | CACHE | DONT_CACHE | SHADE | DONT_SHADE | EMIT_SFX | EXPLODE => 1,
        PUSH_CONSTANT | PUSH_LOCAL | PUSH_STATIC | POP_LOCAL | POP_STATIC | JUMP | JUMP_IF_FALSE => 1,
        START_SCRIPT | CALL_SCRIPT => 2,
        PLAY_SOUND => 1,
        SLEEP | CREATE_LOCAL | POP_STACK | ADD | SUB | MUL | DIV | BITWISE_AND | BITWISE_OR | BITWISE_XOR
        | BITWISE_NOT | RAND | GET_UNIT_VALUE | GET | SET_LESS | SET_LESS_OR_EQUAL | SET_GREATER
        | SET_GREATER_OR_EQUAL | SET_EQUAL | SET_NOT_EQUAL | LOGICAL_AND | LOGICAL_OR | LOGICAL_XOR
        | LOGICAL_NOT | RETURN | SIGNAL | SET_SIGNAL_MASK | SET_VALUE | ATTACH_UNIT | DROP_UNIT => 0,
        _ => return None,
    })
}

pub fn opcode_name(opcode: u32) -> &'static str {
    use op::*;
    match opcode {
        MOVE => "move",
        TURN => "turn",
        SPIN => "spin",
        STOP_SPIN => "stop-spin",
        SHOW => "show",
        HIDE => "hide",
        CACHE => "cache",
        DONT_CACHE => "dont-cache",
        MOVE_NOW => "move-now",
        TURN_NOW => "turn-now",
        SHADE => "shade",
        DONT_SHADE => "dont-shade",
        EMIT_SFX => "emit-sfx",
        WAIT_TURN => "wait-for-turn",
        WAIT_MOVE => "wait-for-move",
        SLEEP => "sleep",
        PUSH_CONSTANT => "push-const",
        PUSH_LOCAL => "push-local",
        PUSH_STATIC => "push-static",
        CREATE_LOCAL => "create-local",
        POP_LOCAL => "pop-local",
        POP_STATIC => "pop-static",
        POP_STACK => "pop-stack",
        ADD => "add",
        SUB => "sub",
        MUL => "mul",
        DIV => "div",
        BITWISE_AND => "and",
        BITWISE_OR => "or",
        BITWISE_XOR => "xor",
        BITWISE_NOT => "not",
        RAND => "rand",
        GET_UNIT_VALUE => "get-unit-value",
        GET => "get",
        SET_LESS => "lt",
        SET_LESS_OR_EQUAL => "le",
        SET_GREATER => "gt",
        SET_GREATER_OR_EQUAL => "ge",
        SET_EQUAL => "eq",
        SET_NOT_EQUAL => "ne",
        LOGICAL_AND => "land",
        LOGICAL_OR => "lor",
        LOGICAL_XOR => "lxor",
        LOGICAL_NOT => "lnot",
        START_SCRIPT => "start-script",
        CALL_SCRIPT => "call-script",
        JUMP => "jump",
        RETURN => "return",
        JUMP_IF_FALSE => "jump-if-false",
        SIGNAL => "signal",
        SET_SIGNAL_MASK => "set-signal-mask",
        EXPLODE => "explode",
        PLAY_SOUND => "play-sound",
        SET_VALUE => "set",
        ATTACH_UNIT => "attach-unit",
        DROP_UNIT => "drop-unit",
        _ => "???",
    }
}

/// Produces a human-readable listing (for tooling and debugging).
pub fn disassemble(script: &CobScript) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    let _ = writeln!(out, "; pieces: {}", script.pieces.join(", "));
    let _ = writeln!(out, "; statics: {}", script.static_var_count);
    let mut starts: Vec<(u32, &str)> = script.functions.iter().map(|f| (f.offset, f.name.as_str())).collect();
    starts.sort();
    let mut pc = 0usize;
    while pc < script.code.len() {
        if let Some((_, name)) = starts.iter().find(|(o, _)| *o as usize == pc) {
            let _ = writeln!(out, "\n{name}:");
        }
        let opcode = script.code[pc];
        let n = operand_count(opcode).unwrap_or(0);
        let args: Vec<String> = script.code[pc + 1..(pc + 1 + n).min(script.code.len())]
            .iter()
            .map(|&a| format!("{}", a as i32))
            .collect();
        let _ = writeln!(out, "  {pc:5}: {:<16} {}", opcode_name(opcode), args.join(" "));
        if operand_count(opcode).is_none() {
            let _ = writeln!(out, "        ; unknown opcode {opcode:#010x}");
        }
        pc += 1 + n;
    }
    out
}
