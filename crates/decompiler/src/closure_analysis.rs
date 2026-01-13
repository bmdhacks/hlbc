//! Closure Analysis Pass
//!
//! Analyzes HashLink bytecode to identify closure patterns and track captured variables.
//!
//! HashLink closures with captured variables are "lowered" to:
//! 1. An **enum/struct** that holds captured values (the "capture context")
//! 2. An **inner function** that takes the enum as its first argument
//! 3. An `InstanceClosure` opcode that binds function + context
//!
//! ## Bytecode Pattern
//!
//! ```text
//! // Parent function: makeAdder(n: Int): Int->Int
//! EnumAlloc     r2 = new <Capture_Context>     // Create capture struct
//! SetEnumField  r2.field_0 = r0                // Store captured 'n'
//! InstanceClosure r1 = r2.inner_func@24        // Bind inner function to context
//! Ret           r1
//!
//! // Inner function: inner_func(ctx: Capture_Context, x: Int): Int
//! EnumField     r3 = (r0 as <Capture_Context>).field_0   // Extract 'n' from context
//! Add           r2 = r1 + r3                              // x + n
//! Ret           r2
//! ```

use std::collections::HashMap;

use hlbc::opcodes::Opcode;
use hlbc::types::{Function, RefEnumConstruct, RefField, RefFun, RefType, Reg};
use hlbc::Bytecode;

/// Information about a single captured variable in a closure
#[derive(Debug, Clone)]
pub struct CapturedVar {
    /// The field index in the capture enum
    pub field_index: RefField,
    /// The source register in the outer function that was captured
    pub source_reg: Reg,
    /// Optional variable name from debug info
    pub name: Option<String>,
}

/// Information about a closure's captured variables
#[derive(Debug, Clone)]
pub struct CaptureInfo {
    /// The outer function that creates the closure
    pub outer_fun: RefFun,
    /// The inner (closure) function
    pub inner_fun: RefFun,
    /// The register holding the capture context (EnumAlloc result) in the outer function
    pub context_reg: Reg,
    /// The enum construct type used for captures
    pub capture_type: RefType,
    /// The enum construct index
    pub capture_construct: RefEnumConstruct,
    /// Maps enum field index -> captured variable info
    pub captures: Vec<CapturedVar>,
    /// Index of the InstanceClosure opcode in the outer function
    pub closure_op_idx: usize,
    /// Index of the EnumAlloc opcode in the outer function
    pub alloc_op_idx: usize,
}

/// Global closure analysis computed once per Bytecode
#[derive(Debug, Default)]
pub struct ClosureAnalysis {
    /// Maps inner function -> its capture info
    pub inner_to_capture: HashMap<RefFun, CaptureInfo>,
    /// Maps outer function -> list of closures it creates
    pub outer_to_closures: HashMap<RefFun, Vec<RefFun>>,
    /// Maps (outer_fun, op_idx) -> inner function for InstanceClosure ops
    pub instance_closure_map: HashMap<(RefFun, usize), RefFun>,
    /// Maps (outer_fun, context_reg) -> inner function for tracking context usage
    pub context_to_inner: HashMap<(RefFun, Reg), RefFun>,
}

impl ClosureAnalysis {
    /// Analyze all functions in bytecode for closure patterns
    pub fn analyze(code: &Bytecode) -> Self {
        let mut analysis = Self::default();

        for func in &code.functions {
            // Use the function's own findex, not the array index
            let outer_fun = func.findex;
            analysis.scan_function(code, outer_fun, func);
        }

        analysis
    }

    /// Check if a function is a closure (has a capture context as first parameter)
    pub fn is_closure(&self, fun: RefFun) -> bool {
        self.inner_to_capture.contains_key(&fun)
    }

    /// Get capture info for a closure function
    pub fn get_capture_info(&self, inner_fun: RefFun) -> Option<&CaptureInfo> {
        self.inner_to_capture.get(&inner_fun)
    }

    /// Check if an opcode index in a function is an InstanceClosure that should be handled specially
    pub fn get_closure_at(&self, outer_fun: RefFun, op_idx: usize) -> Option<&CaptureInfo> {
        let inner = self.instance_closure_map.get(&(outer_fun, op_idx))?;
        self.inner_to_capture.get(inner)
    }

    /// Check if a register in a function holds a closure context
    pub fn is_context_reg(&self, outer_fun: RefFun, reg: Reg) -> bool {
        self.context_to_inner.contains_key(&(outer_fun, reg))
    }

    fn scan_function(&mut self, code: &Bytecode, outer_fun: RefFun, func: &Function) {
        // Track EnumAlloc operations: register -> (op_idx, type_ref, construct)
        let mut enum_allocs: HashMap<Reg, (usize, RefType, RefEnumConstruct)> = HashMap::new();

        // Track SetEnumField operations for each context register
        let mut field_writes: HashMap<Reg, Vec<(RefField, Reg)>> = HashMap::new();

        for (op_idx, op) in func.ops.iter().enumerate() {
            match op {
                Opcode::EnumAlloc { dst, construct } => {
                    // Get the type of the destination register
                    let type_ref = if (dst.0 as usize) < func.regs.len() {
                        func.regs[dst.0 as usize]
                    } else {
                        continue;
                    };

                    // Check if this is an Enum type (used for closure contexts)
                    if let Some(hlbc::types::Type::Enum { .. }) = code.types.get(type_ref.0) {
                        enum_allocs.insert(*dst, (op_idx, type_ref, *construct));
                        field_writes.insert(*dst, Vec::new());
                    }
                }

                Opcode::SetEnumField { value, field, src } => {
                    // Track field writes to potential capture contexts
                    if let Some(writes) = field_writes.get_mut(value) {
                        writes.push((*field, *src));
                    }
                }

                Opcode::InstanceClosure { dst: _, fun, obj } => {
                    // Check if obj is a capture context we've been tracking
                    if let Some((alloc_idx, type_ref, construct)) = enum_allocs.get(obj) {
                        // Found a closure! Build the CaptureInfo
                        let captures = field_writes
                            .get(obj)
                            .map(|writes| {
                                writes
                                    .iter()
                                    .map(|(field, src)| {
                                        // Try to get variable name from debug info
                                        let name = self.get_var_name(code, func, *src);
                                        CapturedVar {
                                            field_index: *field,
                                            source_reg: *src,
                                            name,
                                        }
                                    })
                                    .collect()
                            })
                            .unwrap_or_default();

                        let info = CaptureInfo {
                            outer_fun,
                            inner_fun: *fun,
                            context_reg: *obj,
                            capture_type: *type_ref,
                            capture_construct: *construct,
                            captures,
                            closure_op_idx: op_idx,
                            alloc_op_idx: *alloc_idx,
                        };

                        // Register the closure
                        self.inner_to_capture.insert(*fun, info);
                        self.outer_to_closures
                            .entry(outer_fun)
                            .or_default()
                            .push(*fun);
                        self.instance_closure_map.insert((outer_fun, op_idx), *fun);
                        self.context_to_inner.insert((outer_fun, *obj), *fun);
                    }
                }

                _ => {}
            }
        }
    }

    /// Try to get a variable name from debug info for a register
    fn get_var_name(&self, code: &Bytecode, func: &Function, reg: Reg) -> Option<String> {
        // Check assigns debug info
        if let Some(assigns) = &func.assigns {
            for (name_ref, assign_reg) in assigns {
                // The assign_reg is the register index
                if *assign_reg == reg.0 as usize {
                    if let Some(name) = code.strings.get(name_ref.0) {
                        if !name.is_empty() {
                            return Some(name.to_string());
                        }
                    }
                }
            }
        }
        None
    }
}

/// Analyze inner functions to find which registers access captured variables
#[derive(Debug, Default)]
pub struct InnerFunctionCaptures {
    /// Maps register -> (field_index, original_var_name)
    /// For registers that are loaded from the capture context via EnumField
    pub captured_regs: HashMap<Reg, (RefField, Option<String>)>,
}

impl InnerFunctionCaptures {
    /// Analyze an inner function to find captured variable accesses
    pub fn analyze(_code: &Bytecode, func: &Function, capture_info: &CaptureInfo) -> Self {
        let mut result = Self::default();

        // The capture context is always the first argument (reg0) for inner functions
        let context_reg = Reg(0);

        for op in &func.ops {
            if let Opcode::EnumField {
                dst,
                value,
                construct: _,
                field,
            } = op
            {
                // Check if we're reading from the capture context
                if *value == context_reg {
                    // Find the original variable name from capture info
                    let orig_name = capture_info
                        .captures
                        .iter()
                        .find(|c| c.field_index == *field)
                        .and_then(|c| c.name.clone());

                    result.captured_regs.insert(*dst, (*field, orig_name));
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn test_closure_analysis_empty() {
        // Test with a simple bytecode that has no closures
        // This is a basic sanity test
        let analysis = ClosureAnalysis::default();
        assert!(analysis.inner_to_capture.is_empty());
        assert!(analysis.outer_to_closures.is_empty());
    }

    #[test]
    fn test_closure_analysis_on_closure_test() {
        // Test with the closure test bytecode if available
        // Try multiple paths since cargo runs tests from different directories
        let paths = [
            "tests/roundtrip/bin/closure.hl",
            "../../tests/roundtrip/bin/closure.hl",
        ];
        let path = paths.iter().map(Path::new).find(|p| p.exists());
        let path = match path {
            Some(p) => p,
            None => {
                eprintln!("Skipping test: closure.hl not found");
                return;
            }
        };

        let code = Bytecode::from_file(path).unwrap();
        let analysis = ClosureAnalysis::analyze(&code);

        // The closure.hl file should have closures
        // makeAdder creates a closure with captured 'n'
        // makeMultiplier creates a closure with captured 'n'
        // main creates an inline closure (double)

        // We should find at least 2 closures (makeAdder and makeMultiplier)
        assert!(
            !analysis.inner_to_capture.is_empty(),
            "Expected to find closures in closure.hl"
        );

        // Print debug info
        eprintln!("Found {} closures:", analysis.inner_to_capture.len());
        for (inner, info) in &analysis.inner_to_capture {
            eprintln!(
                "  Closure fun@{}: outer=fun@{}, context_reg=r{}, {} captures",
                inner.0,
                info.outer_fun.0,
                info.context_reg.0,
                info.captures.len()
            );
            for cap in &info.captures {
                eprintln!(
                    "    field_{} <- r{} (name: {:?})",
                    cap.field_index.0, cap.source_reg.0, cap.name
                );
            }
        }
    }
}
