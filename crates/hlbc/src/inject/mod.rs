//! Bytecode injection and patching module.
//!
//! This module provides tools for:
//! - Inserting call opcodes at specific positions
//! - Adjusting jump offsets after opcode insertion
//!
//! # Example
//!
//! ```ignore
//! use hlbc::Bytecode;
//! use hlbc::inject::{FunctionPatcher, CallSpec};
//!
//! let mut target = Bytecode::from_file("target.hl")?;
//!
//! // Insert a call at a specific location
//! let mut patcher = FunctionPatcher::new(&mut target, "Main.init")?;
//! patcher.insert_call_at(10, CallSpec::call0(some_fun))?;
//! ```

mod jumps;
pub mod matching;

use crate::opcodes::Opcode;
use crate::types::{Function, RefFun, RefType, Reg};
use crate::Bytecode;

pub use jumps::{adjust_jumps_after_insert, adjust_jumps_after_remove};
pub use matching::{matches_pattern, FunctionIndex, QualifiedName};

/// Error type for injection and patching operations
#[derive(Debug, Clone)]
pub enum InjectionError {
    /// Function not found in source bytecode
    FunctionNotFoundInSource(String),
    /// Function not found in target bytecode
    FunctionNotFoundInTarget(String),
    /// Opcode index is out of bounds
    OpcodeIndexOutOfBounds { index: usize, count: usize },
    /// Register is not available in the function
    RegisterNotAvailable { reg: u32, count: usize },
    /// Cannot insert call with non-void return without destination register
    MissingDestinationRegister,
    /// Type mismatch detected
    TypeMismatch { description: String },
}

impl std::fmt::Display for InjectionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InjectionError::FunctionNotFoundInSource(name) => {
                write!(f, "Function '{}' not found in source bytecode", name)
            }
            InjectionError::FunctionNotFoundInTarget(name) => {
                write!(f, "Function '{}' not found in target bytecode", name)
            }
            InjectionError::OpcodeIndexOutOfBounds { index, count } => {
                write!(f, "Opcode index {} out of bounds (function has {} opcodes)", index, count)
            }
            InjectionError::RegisterNotAvailable { reg, count } => {
                write!(f, "Register {} not available (function has {} registers)", reg, count)
            }
            InjectionError::MissingDestinationRegister => {
                write!(f, "Cannot insert call with non-void return without destination register")
            }
            InjectionError::TypeMismatch { description } => {
                write!(f, "Type mismatch: {}", description)
            }
        }
    }
}

impl std::error::Error for InjectionError {}

/// Specification for a call opcode to insert.
#[derive(Debug, Clone)]
pub struct CallSpec {
    /// Function to call
    pub fun: RefFun,
    /// Arguments (registers in the target function)
    pub args: Vec<Reg>,
    /// Destination register for return value (None for void returns, uses Reg(0))
    pub dst: Option<Reg>,
}

impl CallSpec {
    /// Create a Call0 spec (no arguments).
    ///
    /// For void-returning functions, dst can be None (will use Reg(0)).
    pub fn call0(fun: RefFun) -> Self {
        Self {
            fun,
            args: vec![],
            dst: None,
        }
    }

    /// Create a Call0 spec with explicit destination register.
    pub fn call0_with_dst(fun: RefFun, dst: Reg) -> Self {
        Self {
            fun,
            args: vec![],
            dst: Some(dst),
        }
    }

    /// Create a Call1 spec.
    pub fn call1(fun: RefFun, arg0: Reg) -> Self {
        Self {
            fun,
            args: vec![arg0],
            dst: None,
        }
    }

    /// Create a Call1 spec with explicit destination register.
    pub fn call1_with_dst(fun: RefFun, arg0: Reg, dst: Reg) -> Self {
        Self {
            fun,
            args: vec![arg0],
            dst: Some(dst),
        }
    }

    /// Create a Call2 spec.
    pub fn call2(fun: RefFun, arg0: Reg, arg1: Reg) -> Self {
        Self {
            fun,
            args: vec![arg0, arg1],
            dst: None,
        }
    }

    /// Create a CallN spec (N arguments).
    pub fn call_n(fun: RefFun, args: Vec<Reg>) -> Self {
        Self {
            fun,
            args,
            dst: None,
        }
    }

    /// Create a CallN spec with explicit destination register.
    pub fn call_n_with_dst(fun: RefFun, args: Vec<Reg>, dst: Reg) -> Self {
        Self {
            fun,
            args,
            dst: Some(dst),
        }
    }

    /// Convert to an Opcode.
    fn to_opcode(&self) -> Opcode {
        let dst = self.dst.unwrap_or(Reg(0));
        match self.args.len() {
            0 => Opcode::Call0 { dst, fun: self.fun },
            1 => Opcode::Call1 { dst, fun: self.fun, arg0: self.args[0] },
            2 => Opcode::Call2 {
                dst,
                fun: self.fun,
                arg0: self.args[0],
                arg1: self.args[1],
            },
            3 => Opcode::Call3 {
                dst,
                fun: self.fun,
                arg0: self.args[0],
                arg1: self.args[1],
                arg2: self.args[2],
            },
            4 => Opcode::Call4 {
                dst,
                fun: self.fun,
                arg0: self.args[0],
                arg1: self.args[1],
                arg2: self.args[2],
                arg3: self.args[3],
            },
            _ => Opcode::CallN {
                dst,
                fun: self.fun,
                args: self.args.clone(),
            },
        }
    }
}

/// Function patcher - modifies opcodes in existing functions.
///
/// Provides methods to insert, replace, and remove opcodes with automatic
/// jump offset adjustment.
pub struct FunctionPatcher<'a> {
    bytecode: &'a mut Bytecode,
    func_idx: usize,
}

impl<'a> FunctionPatcher<'a> {
    /// Create a patcher for a function by qualified name.
    ///
    /// # Arguments
    /// * `bytecode` - Bytecode to modify
    /// * `func_name` - Qualified function name (e.g., "Main.init")
    pub fn new(bytecode: &'a mut Bytecode, func_name: &str) -> Result<Self, InjectionError> {
        let index = FunctionIndex::build(bytecode);
        let func_idx = index.find(func_name)
            .ok_or_else(|| InjectionError::FunctionNotFoundInTarget(func_name.to_string()))?;

        Ok(Self { bytecode, func_idx })
    }

    /// Create a patcher for a function by its index in the functions array.
    pub fn from_func_index(bytecode: &'a mut Bytecode, func_idx: usize) -> Result<Self, InjectionError> {
        if func_idx >= bytecode.functions.len() {
            return Err(InjectionError::FunctionNotFoundInTarget(format!("index {}", func_idx)));
        }
        Ok(Self { bytecode, func_idx })
    }

    /// Create a patcher for a function by its findex (RefFun).
    ///
    /// Note: This only works for non-native functions.
    pub fn from_findex(bytecode: &'a mut Bytecode, findex: RefFun) -> Result<Self, InjectionError> {
        // Find the function with this findex
        let func_idx = bytecode.functions
            .iter()
            .position(|f| f.findex == findex)
            .ok_or_else(|| InjectionError::FunctionNotFoundInTarget(format!("findex {}", findex.0)))?;

        Ok(Self { bytecode, func_idx })
    }

    /// Get a reference to the function being patched.
    pub fn function(&self) -> &Function {
        &self.bytecode.functions[self.func_idx]
    }

    /// Get the number of opcodes in the function.
    pub fn opcode_count(&self) -> usize {
        self.function().ops.len()
    }

    /// Insert a call at the specified opcode index.
    ///
    /// All subsequent opcodes are shifted, and jump offsets are adjusted automatically.
    ///
    /// # Arguments
    /// * `index` - Index where the call should be inserted (0 = before first opcode)
    /// * `call` - Call specification
    pub fn insert_call_at(&mut self, index: usize, call: CallSpec) -> Result<(), InjectionError> {
        self.insert_opcodes_at(index, vec![call.to_opcode()])
    }

    /// Insert multiple opcodes at the specified index.
    ///
    /// All subsequent opcodes are shifted, and jump offsets are adjusted automatically.
    ///
    /// # Arguments
    /// * `index` - Index where opcodes should be inserted
    /// * `ops` - Opcodes to insert
    pub fn insert_opcodes_at(&mut self, index: usize, ops: Vec<Opcode>) -> Result<(), InjectionError> {
        let count = self.opcode_count();
        if index > count {
            return Err(InjectionError::OpcodeIndexOutOfBounds { index, count });
        }

        let inserted_count = ops.len() as i32;

        // Insert the opcodes
        let func = &mut self.bytecode.functions[self.func_idx];
        for (i, op) in ops.into_iter().enumerate() {
            func.ops.insert(index + i, op);
        }

        // Adjust jump offsets
        adjust_jumps_after_insert(&mut func.ops, index, inserted_count);

        // Adjust debug info if present
        if let Some(ref mut debug_info) = func.debug_info {
            // Insert dummy debug entries for the new opcodes
            for i in 0..inserted_count as usize {
                debug_info.insert(index + i, (0, 0));
            }
        }

        Ok(())
    }

    /// Replace an opcode at the specified index.
    ///
    /// This does not shift opcodes or adjust jump offsets.
    ///
    /// # Arguments
    /// * `index` - Index of the opcode to replace
    /// * `op` - New opcode
    pub fn replace_opcode_at(&mut self, index: usize, op: Opcode) -> Result<(), InjectionError> {
        let count = self.opcode_count();
        if index >= count {
            return Err(InjectionError::OpcodeIndexOutOfBounds { index, count });
        }

        self.bytecode.functions[self.func_idx].ops[index] = op;
        Ok(())
    }

    /// Remove an opcode at the specified index.
    ///
    /// All subsequent opcodes are shifted, and jump offsets are adjusted automatically.
    ///
    /// # Arguments
    /// * `index` - Index of the opcode to remove
    pub fn remove_opcode_at(&mut self, index: usize) -> Result<Opcode, InjectionError> {
        let count = self.opcode_count();
        if index >= count {
            return Err(InjectionError::OpcodeIndexOutOfBounds { index, count });
        }

        let func = &mut self.bytecode.functions[self.func_idx];

        // Remove the opcode
        let removed = func.ops.remove(index);

        // Adjust jump offsets (negative delta since we removed)
        adjust_jumps_after_remove(&mut func.ops, index, 1);

        // Adjust debug info if present
        if let Some(ref mut debug_info) = func.debug_info {
            if index < debug_info.len() {
                debug_info.remove(index);
            }
        }

        Ok(removed)
    }

    /// Get access to the opcode at the specified index.
    pub fn get_opcode(&self, index: usize) -> Option<&Opcode> {
        self.function().ops.get(index)
    }

    /// Get mutable access to the opcode at the specified index.
    pub fn get_opcode_mut(&mut self, index: usize) -> Option<&mut Opcode> {
        self.bytecode.functions[self.func_idx].ops.get_mut(index)
    }

    /// Get the function's register types (useful for determining available registers).
    pub fn register_types(&self) -> &[RefType] {
        &self.function().regs
    }
}
