//! COB (compiled BOS unit script) reader, disassembler, writer ([`CobScript::to_bytes`]) and a
//! small assembler ([`Assembler`]: the scripts TA Reborn makes for other games' units).
//!
//! Each unit has a script that animates pieces and answers engine queries (aiming, build
//! positions, ...). Header (TA = version 4, 11 × i32): version, script count, piece count,
//! code length (in u32 words), static variable count, 0, script code-offset table ptr,
//! script name-ptr table ptr, piece name-ptr table ptr, code ptr, 0. Version 6 (TA: Kingdoms',
//! as the Spring engine reads it) adds two: the sound name-ptr table ptr and the sound count, the
//! names `play-sound` plays by their numbers ([`CobScript::sounds`]; a script with sounds is
//! written so).

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
    /// The sounds `play-sound` names by their numbers (version 6's table; none in TA's own).
    pub sounds: Vec<String>,
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
        // (Version 6's sounds, where its header has room for their table: a table that does not
        // read leaves the script without sounds, never unread.)
        let mut sounds = Vec::new();
        if version == 6 && data.len() >= 52 {
            let (sounds_ptr, sound_count) = (i32_at(data, 44)?, i32_at(data, 48)?);
            if sounds_ptr > 0 && (1..=4096).contains(&sound_count) && table(data, sounds_ptr as usize, sound_count as usize, 4).is_ok() {
                for i in 0..sound_count as usize {
                    match i32_at(data, sounds_ptr as usize + i * 4).ok().and_then(|at| cstr_at(data, at.max(0) as usize).ok()) {
                        Some(name) => {
                            budget.take(name.len() + 24, "COB")?;
                            sounds.push(name);
                        }
                        None => {
                            sounds.clear();
                            break;
                        }
                    }
                }
            }
        }
        Ok(CobScript { functions, pieces, code, static_var_count, sounds })
    }

    /// Case-insensitive function lookup.
    pub fn function(&self, name: &str) -> Option<usize> {
        self.functions.iter().position(|f| f.name.eq_ignore_ascii_case(name))
    }

    /// The script as a COB file of version 4 (6 with sounds, their table after the pieces'), as
    /// [`CobScript::parse`] reads one: the header, the code, the functions' entry points, the
    /// pointers to their names and to the pieces' (and the sounds'), then the names.
    pub fn to_bytes(&self) -> Vec<u8> {
        let with_sounds = !self.sounds.is_empty();
        let header = if with_sounds { 52usize } else { 44 };
        let code_at = header;
        let offsets_at = code_at + self.code.len() * 4;
        let names_at = offsets_at + self.functions.len() * 4;
        let pieces_at = names_at + self.functions.len() * 4;
        let sounds_at = pieces_at + self.pieces.len() * 4;
        let strings_at = sounds_at + self.sounds.len() * 4;
        let mut strings: Vec<u8> = Vec::new();
        let mut string = |s: &str| -> u32 {
            let at = (strings_at + strings.len()) as u32;
            strings.extend(s.bytes().map(|b| if b == 0 { b'?' } else { b }));
            strings.push(0);
            at
        };
        let names: Vec<u32> = self.functions.iter().map(|f| string(&f.name)).collect();
        let pieces: Vec<u32> = self.pieces.iter().map(|p| string(p)).collect();
        let sounds: Vec<u32> = self.sounds.iter().map(|s| string(s)).collect();
        let mut out = Vec::with_capacity(strings_at + strings.len());
        for v in [if with_sounds { 6 } else { 4 }, self.functions.len(), self.pieces.len(), self.code.len(), self.static_var_count as usize, 0, offsets_at, names_at, pieces_at, code_at, 0] {
            out.extend_from_slice(&(v as u32).to_le_bytes());
        }
        if with_sounds {
            out.extend_from_slice(&(sounds_at as u32).to_le_bytes());
            out.extend_from_slice(&(self.sounds.len() as u32).to_le_bytes());
        }
        for w in &self.code {
            out.extend_from_slice(&w.to_le_bytes());
        }
        for f in &self.functions {
            out.extend_from_slice(&f.offset.to_le_bytes());
        }
        for at in names.iter().chain(&pieces).chain(&sounds) {
            out.extend_from_slice(&at.to_le_bytes());
        }
        out.extend_from_slice(&strings);
        out
    }
}

/// The axes, as scripts name them.
pub const X_AXIS: u32 = 0;
pub const Y_AXIS: u32 = 1;
pub const Z_AXIS: u32 = 2;

/// A place in a function's code a jump goes to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Label(usize);

/// A function's code as it is written, a word at a time: its parameters claimed first (the
/// callee's leading `create-local`s), then what the methods add. Jumps go to [`Label`]s, made
/// absolute when the function takes its place in the script ([`Assembler::define`]).
#[derive(Clone, Debug, Default)]
pub struct Function {
    code: Vec<u32>,
    labels: Vec<Option<u32>>,
    /// (Where a jump's operand is, the label it names.)
    jumps: Vec<(usize, usize)>,
    locals: u32,
    /// Where the last instruction starts (its opcode).
    last: Option<usize>,
}

impl Function {
    /// A function taking `params` arguments: locals 0 to `params` - 1.
    pub fn new(params: u32) -> Function {
        let mut f = Function::default();
        for _ in 0..params {
            f.local();
        }
        f
    }

    /// A new local (its index).
    pub fn local(&mut self) -> u32 {
        self.emit(&[op::CREATE_LOCAL]);
        self.locals += 1;
        self.locals - 1
    }

    fn emit(&mut self, words: &[u32]) -> &mut Self {
        self.last = Some(self.code.len());
        self.code.extend_from_slice(words);
        self
    }

    /// Whether running off its end can happen: its last instruction is no `return`, or a jump
    /// can land past it (a label marked at its very end). An operand of the value of `return`'s
    /// opcode is no return: the last instruction is told by where it starts.
    fn runs_off_its_end(&self) -> bool {
        let end = self.code.len() as u32;
        self.last.is_none_or(|at| self.code[at] != op::RETURN) || self.labels.iter().any(|&l| l == Some(end))
    }

    /// An opcode of no operands (arithmetic, comparisons, `return`, `signal`...: on the stack).
    pub fn op(&mut self, opcode: u32) -> &mut Self {
        self.emit(&[opcode])
    }
    pub fn push(&mut self, value: i32) -> &mut Self {
        self.emit(&[op::PUSH_CONSTANT, value as u32])
    }
    pub fn push_local(&mut self, local: u32) -> &mut Self {
        self.emit(&[op::PUSH_LOCAL, local])
    }
    pub fn pop_local(&mut self, local: u32) -> &mut Self {
        self.emit(&[op::POP_LOCAL, local])
    }
    pub fn push_static(&mut self, var: u32) -> &mut Self {
        self.emit(&[op::PUSH_STATIC, var])
    }
    pub fn pop_static(&mut self, var: u32) -> &mut Self {
        self.emit(&[op::POP_STATIC, var])
    }
    /// `turn <piece> to <axis> <target> speed <speed>`: the speed, then the target, on the stack.
    pub fn turn(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::TURN, piece, axis])
    }
    /// `turn <piece> to <axis> <target> now`: the target on the stack.
    pub fn turn_now(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::TURN_NOW, piece, axis])
    }
    /// `move <piece> to <axis> <target> speed <speed>` (16.16 pixels): as [`Function::turn`].
    pub fn move_to(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::MOVE, piece, axis])
    }
    pub fn move_now(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::MOVE_NOW, piece, axis])
    }
    pub fn wait_turn(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::WAIT_TURN, piece, axis])
    }
    pub fn wait_move(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::WAIT_MOVE, piece, axis])
    }
    /// `spin <piece> around <axis> speed <s> accelerate <a>`: the acceleration, then the speed.
    pub fn spin(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::SPIN, piece, axis])
    }
    /// `stop-spin <piece> around <axis> decelerate <d>`: the deceleration on the stack (0: at once).
    pub fn stop_spin(&mut self, piece: u32, axis: u32) -> &mut Self {
        self.emit(&[op::STOP_SPIN, piece, axis])
    }
    pub fn show(&mut self, piece: u32) -> &mut Self {
        self.emit(&[op::SHOW, piece])
    }
    pub fn hide(&mut self, piece: u32) -> &mut Self {
        self.emit(&[op::HIDE, piece])
    }
    /// `play-sound` of the script's sound `sound` (its number, [`Assembler::sound`]): nothing
    /// taken from the stack, as TA Reborn's engine runs it.
    pub fn play_sound(&mut self, sound: u32) -> &mut Self {
        self.emit(&[op::PLAY_SOUND, sound])
    }
    /// `emit-sfx <kind> from <piece>`: the kind on the stack.
    pub fn emit_sfx(&mut self, piece: u32) -> &mut Self {
        self.emit(&[op::EMIT_SFX, piece])
    }
    /// `explode <piece> type <flags>`: the flags on the stack.
    pub fn explode(&mut self, piece: u32) -> &mut Self {
        self.emit(&[op::EXPLODE, piece])
    }
    /// `start-script <function>(<argc> arguments on the stack)`.
    pub fn start_script(&mut self, function: u32, argc: u32) -> &mut Self {
        self.emit(&[op::START_SCRIPT, function, argc])
    }
    /// `call-script <function>(<argc> arguments on the stack)`.
    pub fn call_script(&mut self, function: u32, argc: u32) -> &mut Self {
        self.emit(&[op::CALL_SCRIPT, function, argc])
    }
    /// `return <value>`.
    pub fn ret(&mut self, value: i32) -> &mut Self {
        self.push(value).op(op::RETURN)
    }
    /// A label, to [`Function::mark`] somewhere and jump to.
    pub fn label(&mut self) -> Label {
        self.labels.push(None);
        Label(self.labels.len() - 1)
    }
    /// The label is here.
    pub fn mark(&mut self, label: Label) -> &mut Self {
        self.labels[label.0] = Some(self.code.len() as u32);
        self
    }
    pub fn jump(&mut self, label: Label) -> &mut Self {
        self.jumps.push((self.code.len() + 1, label.0));
        self.emit(&[op::JUMP, 0])
    }
    /// Jumps when the value on the stack is 0 (popping it).
    pub fn jump_if_false(&mut self, label: Label) -> &mut Self {
        self.jumps.push((self.code.len() + 1, label.0));
        self.emit(&[op::JUMP_IF_FALSE, 0])
    }
}

/// A script made a function at a time: its pieces named up front, functions declared (so any
/// may start or call any) and defined, static variables taken as needed.
#[derive(Clone, Debug, Default)]
pub struct Assembler {
    pieces: Vec<String>,
    names: Vec<String>,
    bodies: Vec<Option<Function>>,
    statics: u32,
    sounds: Vec<String>,
}

impl Assembler {
    pub fn new(pieces: Vec<String>) -> Assembler {
        Assembler { pieces, ..Default::default() }
    }

    /// A piece's number (any case).
    pub fn piece(&self, name: &str) -> Option<u32> {
        self.pieces.iter().position(|p| p.eq_ignore_ascii_case(name)).map(|i| i as u32)
    }

    /// A sound's number for `play-sound` (any case), named in the script if it is not yet.
    pub fn sound(&mut self, name: &str) -> u32 {
        match self.sounds.iter().position(|s| s.eq_ignore_ascii_case(name)) {
            Some(i) => i as u32,
            None => {
                self.sounds.push(name.to_string());
                (self.sounds.len() - 1) as u32
            }
        }
    }

    /// A new static variable (its number).
    pub fn static_var(&mut self) -> u32 {
        self.statics += 1;
        self.statics - 1
    }

    /// A function's number, declared if it is not yet.
    pub fn declare(&mut self, name: &str) -> u32 {
        match self.names.iter().position(|n| n.eq_ignore_ascii_case(name)) {
            Some(i) => i as u32,
            None => {
                self.names.push(name.to_string());
                self.bodies.push(None);
                (self.names.len() - 1) as u32
            }
        }
    }

    /// Gives the function named `name` its code (declaring it if need be).
    pub fn define(&mut self, name: &str, body: Function) -> u32 {
        let i = self.declare(name);
        self.bodies[i as usize] = Some(body);
        i
    }

    /// The script: the functions' code one after another, each's jumps made absolute. A function
    /// declared and never defined returns 0. Panics on a jump to a label never marked.
    pub fn finish(self) -> CobScript {
        let mut code: Vec<u32> = Vec::new();
        let mut functions = Vec::with_capacity(self.names.len());
        for (name, body) in self.names.into_iter().zip(self.bodies) {
            let base = code.len() as u32;
            let mut body = body.unwrap_or_else(|| {
                let mut f = Function::new(0);
                f.ret(0);
                f
            });
            for &(at, label) in &body.jumps {
                body.code[at] = base + body.labels[label].unwrap_or_else(|| panic!("{name}: a jump to a label never marked"));
            }
            // (A function that runs off its end returns 0: past its last instruction, or from a
            // jump to its very end, it would run on into the next function's code.)
            if body.runs_off_its_end() {
                body.ret(0);
            }
            code.extend_from_slice(&body.code);
            functions.push(CobFunction { name, offset: base });
        }
        CobScript { functions, pieces: self.pieces, code, static_var_count: self.statics, sounds: self.sounds }
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

#[cfg(test)]
mod tests {
    use super::*;

    /// An assembled script written as a COB file reads back the same: its functions where they
    /// start, its pieces, its statics, its jumps made absolute.
    #[test]
    fn an_assembled_script_writes_and_reads_back() {
        let mut asm = Assembler::new(vec!["base".into(), "turret".into()]);
        let restore = asm.declare("Restore");
        let flag = asm.static_var();
        let mut create = Function::new(0);
        create.push(1).pop_static(flag).ret(0);
        asm.define("Create", create);
        let mut aim = Function::new(2);
        let skip = aim.label();
        aim.push_static(flag).jump_if_false(skip);
        aim.push(1000).push_local(0).turn(1, Y_AXIS).wait_turn(1, Y_AXIS);
        aim.start_script(restore, 0).mark(skip).ret(1);
        asm.define("AimPrimary", aim);
        let script = asm.finish();
        let back = CobScript::parse(&script.to_bytes()).unwrap();
        assert_eq!(back.pieces, ["base", "turret"]);
        assert_eq!(back.static_var_count, 1);
        assert_eq!(back.functions.iter().map(|f| (f.name.as_str(), f.offset)).collect::<Vec<_>>(), script.functions.iter().map(|f| (f.name.as_str(), f.offset)).collect::<Vec<_>>());
        assert_eq!(back.code, script.code);
        // Restore, declared first and never defined, returns 0; Create starts after it.
        assert_eq!(back.function("restore"), Some(0));
        assert_eq!(&back.code[..3], &[op::PUSH_CONSTANT, 0, op::RETURN]);
        // The aim's jump lands on its last push: the function's start plus its place in it.
        let aim = back.functions[back.function("AimPrimary").unwrap()].offset as usize;
        let jump = (aim..back.code.len()).find(|&i| back.code[i] == op::JUMP_IF_FALSE).unwrap();
        let target = back.code[jump + 1] as usize;
        assert_eq!(&back.code[target..target + 3], &[op::PUSH_CONSTANT, 1, op::RETURN]);
        assert!(disassemble(&back).contains("AimPrimary:"));
        assert!(back.sounds.is_empty() && script.to_bytes()[0] == 4, "TA's version without sounds");
    }

    /// A script's sounds (version 6's table, as TA: Kingdoms' scripts carry theirs): numbered as
    /// first named, any case, written and read back with its play-sounds.
    #[test]
    fn a_scripts_sounds_are_written_and_read_back() {
        let mut asm = Assembler::new(vec!["base".into()]);
        let open = asm.sound("Doors_Open");
        let close = asm.sound("doors_close");
        assert_eq!((open, close, asm.sound("DOORS_OPEN")), (0, 1, 0));
        let mut f = Function::new(0);
        f.play_sound(close).play_sound(open).ret(0);
        asm.define("Activate", f);
        let script = asm.finish();
        let bytes = script.to_bytes();
        assert_eq!(bytes[0], 6);
        let back = CobScript::parse(&bytes).unwrap();
        assert_eq!(back.sounds, ["Doors_Open", "doors_close"]);
        assert_eq!(back.pieces, ["base"]);
        assert_eq!(&back.code[..4], &[op::PLAY_SOUND, 1, op::PLAY_SOUND, 0]);
        assert!(disassemble(&back).contains("play-sound"));
        // (A table that does not read leaves the script without sounds.)
        let mut broken = bytes.clone();
        broken[44..48].copy_from_slice(&(bytes.len() as u32 + 64).to_le_bytes());
        assert!(CobScript::parse(&broken).unwrap().sounds.is_empty());
    }

    /// A function never runs on into the next one's code: one whose last word only looks like a
    /// `return` (an operand of its value) returns 0 after it, and so does one with a label marked
    /// at its very end, after a `return`, which a jump would otherwise carry into the next
    /// function. One that ends in a `return` no jump goes past is left as it is.
    #[test]
    fn a_function_never_runs_on_into_the_next() {
        let mut asm = Assembler::new(vec![]);
        // Its last instruction pushes the value of `return`'s opcode: no return.
        let mut looks = Function::new(0);
        looks.push(op::RETURN as i32);
        asm.define("Looks", looks);
        // `if (x) { return 1; }` with the label after the return, at the very end.
        let mut ends = Function::new(1);
        let after = ends.label();
        ends.push_local(0).jump_if_false(after).ret(1).mark(after);
        asm.define("Ends", ends);
        // Ends in a return, as most do.
        let mut whole = Function::new(0);
        whole.ret(2);
        asm.define("Whole", whole);
        let script = asm.finish();
        let start = |name: &str| script.functions[script.function(name).unwrap()].offset as usize;
        let (looks, ends, whole) = (start("Looks"), start("Ends"), start("Whole"));
        assert_eq!(&script.code[looks..ends], &[op::PUSH_CONSTANT, op::RETURN, op::PUSH_CONSTANT, 0, op::RETURN], "a return after the push");
        // The jump lands on the return of 0 added at the end, not on the next function's code.
        let jump = (ends..whole).find(|&i| script.code[i] == op::JUMP_IF_FALSE).unwrap();
        let target = script.code[jump + 1] as usize;
        assert!(target < whole, "within its own function: {target} (the next starts at {whole})");
        assert_eq!(&script.code[target..target + 3], &[op::PUSH_CONSTANT, 0, op::RETURN]);
        assert_eq!(&script.code[whole..], &[op::PUSH_CONSTANT, 2, op::RETURN], "nothing added");
    }
}
