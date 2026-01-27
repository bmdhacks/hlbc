use std::collections::{HashMap, HashSet};

use hlbc::opcodes::Opcode;
use hlbc::types::{
    RefBytes, RefEnumConstruct, RefField, RefFloat, RefFun, RefGlobal, RefInt, RefString, RefType,
};

/// Mapping from source bytecode indexes to target bytecode indexes
#[derive(Debug, Clone, Default)]
pub struct IndexRemap {
    pub ints: HashMap<usize, usize>,
    pub floats: HashMap<usize, usize>,
    pub strings: HashMap<usize, usize>,
    pub bytes: HashMap<usize, usize>,
    pub types: HashMap<usize, usize>,
    pub globals: HashMap<usize, usize>,
    pub funs: HashMap<usize, usize>,
    pub enum_constructs: HashMap<usize, usize>,
    /// Per-type field index remapping: source_type_idx -> (src_field_idx -> target_field_idx)
    pub type_fields: HashMap<usize, HashMap<usize, usize>>,
    /// Per-enum-type construct index remapping: source_enum_type_idx -> (src_construct_idx -> target_construct_idx)
    pub enum_type_constructs: HashMap<usize, HashMap<usize, usize>>,
    /// Target construct count for each enum type: source_type_idx -> target_construct_count
    /// Used for Switch opcode remapping to size the offsets vector correctly
    pub enum_type_target_counts: HashMap<usize, usize>,
    /// Fields in source types that don't exist in target: type_idx -> set of field indices
    /// Used for validation during opcode remapping
    pub missing_fields: HashMap<usize, HashSet<usize>>,
}

impl IndexRemap {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn remap_int(&self, src: RefInt) -> RefInt {
        RefInt(*self.ints.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_float(&self, src: RefFloat) -> RefFloat {
        RefFloat(*self.floats.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_string(&self, src: RefString) -> RefString {
        RefString(*self.strings.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_bytes(&self, src: RefBytes) -> RefBytes {
        RefBytes(*self.bytes.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_type(&self, src: RefType) -> RefType {
        // Known types (primitives) don't need remapping
        if src.is_known() {
            return src;
        }
        RefType(*self.types.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_global(&self, src: RefGlobal) -> RefGlobal {
        RefGlobal(*self.globals.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_fun(&self, src: RefFun) -> RefFun {
        RefFun(*self.funs.get(&src.0).unwrap_or(&src.0))
    }

    pub fn remap_enum_construct(&self, src: RefEnumConstruct) -> RefEnumConstruct {
        RefEnumConstruct(*self.enum_constructs.get(&src.0).unwrap_or(&src.0))
    }

    /// Remap a field index based on the source type
    /// Uses the type_fields map to translate source field indices to target indices
    pub fn remap_field(&self, src_type_idx: usize, src_field: RefField) -> RefField {
        if let Some(field_map) = self.type_fields.get(&src_type_idx) {
            if let Some(&target_idx) = field_map.get(&src_field.0) {
                return RefField(target_idx);
            }
        }
        src_field // No remap found, return as-is
    }

    /// Check if a field access would touch a field missing in target.
    /// Returns Some(error_message) if invalid, None if OK.
    pub fn validate_field_access(&self, src_type_idx: usize, src_field: RefField) -> Option<String> {
        if let Some(missing) = self.missing_fields.get(&src_type_idx) {
            if missing.contains(&src_field.0) {
                return Some(format!(
                    "Injected code accesses field {} on type {} which doesn't exist in target",
                    src_field.0, src_type_idx
                ));
            }
        }
        None
    }

    /// Remap an enum construct index based on the source enum type
    /// Uses the enum_type_constructs map to translate source construct indices to target indices
    pub fn remap_enum_construct_for_type(
        &self,
        src_type_idx: usize,
        src_construct: RefEnumConstruct,
    ) -> RefEnumConstruct {
        if let Some(construct_map) = self.enum_type_constructs.get(&src_type_idx) {
            if let Some(&target_idx) = construct_map.get(&src_construct.0) {
                return RefEnumConstruct(target_idx);
            }
        }
        src_construct // No remap found, return as-is
    }

    /// Remap all pool references in an opcode
    pub fn remap_opcode(&self, op: &Opcode) -> Opcode {
        match op.clone() {
            // Constant pool references
            Opcode::Int { dst, ptr } => Opcode::Int {
                dst,
                ptr: self.remap_int(ptr),
            },
            Opcode::Float { dst, ptr } => Opcode::Float {
                dst,
                ptr: self.remap_float(ptr),
            },
            Opcode::Bytes { dst, ptr } => Opcode::Bytes {
                dst,
                ptr: self.remap_bytes(ptr),
            },
            Opcode::String { dst, ptr } => Opcode::String {
                dst,
                ptr: self.remap_string(ptr),
            },

            // Function references
            Opcode::Call0 { dst, fun } => Opcode::Call0 {
                dst,
                fun: self.remap_fun(fun),
            },
            Opcode::Call1 { dst, fun, arg0 } => Opcode::Call1 {
                dst,
                fun: self.remap_fun(fun),
                arg0,
            },
            Opcode::Call2 {
                dst,
                fun,
                arg0,
                arg1,
            } => Opcode::Call2 {
                dst,
                fun: self.remap_fun(fun),
                arg0,
                arg1,
            },
            Opcode::Call3 {
                dst,
                fun,
                arg0,
                arg1,
                arg2,
            } => Opcode::Call3 {
                dst,
                fun: self.remap_fun(fun),
                arg0,
                arg1,
                arg2,
            },
            Opcode::Call4 {
                dst,
                fun,
                arg0,
                arg1,
                arg2,
                arg3,
            } => Opcode::Call4 {
                dst,
                fun: self.remap_fun(fun),
                arg0,
                arg1,
                arg2,
                arg3,
            },
            Opcode::CallN { dst, fun, args } => Opcode::CallN {
                dst,
                fun: self.remap_fun(fun),
                args,
            },
            Opcode::StaticClosure { dst, fun } => Opcode::StaticClosure {
                dst,
                fun: self.remap_fun(fun),
            },
            Opcode::InstanceClosure { dst, fun, obj } => Opcode::InstanceClosure {
                dst,
                fun: self.remap_fun(fun),
                obj,
            },

            // Global references
            Opcode::GetGlobal { dst, global } => Opcode::GetGlobal {
                dst,
                global: self.remap_global(global),
            },
            Opcode::SetGlobal { global, src } => Opcode::SetGlobal {
                global: self.remap_global(global),
                src,
            },

            // Type references
            Opcode::Type { dst, ty } => Opcode::Type {
                dst,
                ty: self.remap_type(ty),
            },

            // Dynamic field access uses strings
            Opcode::DynGet { dst, obj, field } => Opcode::DynGet {
                dst,
                obj,
                field: self.remap_string(field),
            },
            Opcode::DynSet { obj, field, src } => Opcode::DynSet {
                obj,
                field: self.remap_string(field),
                src,
            },

            // Enum construction
            Opcode::MakeEnum {
                dst,
                construct,
                args,
            } => Opcode::MakeEnum {
                dst,
                construct: self.remap_enum_construct(construct),
                args,
            },
            Opcode::EnumAlloc { dst, construct } => Opcode::EnumAlloc {
                dst,
                construct: self.remap_enum_construct(construct),
            },
            Opcode::EnumField {
                dst,
                value,
                construct,
                field,
            } => Opcode::EnumField {
                dst,
                value,
                construct: self.remap_enum_construct(construct),
                field,
            },

            // All other opcodes don't reference pools (only registers, jumps, fields)
            // Fields (RefField) are relative to the object type, not global
            other => other,
        }
    }

    /// Remap opcode with field remapping support
    /// Takes register types from the source function to determine which type each field access is for
    pub fn remap_opcode_with_regs(&self, op: &Opcode, src_regs: &[RefType]) -> Opcode {
        match op.clone() {
            // Field access opcodes - need to remap field indices based on object type
            Opcode::Field { dst, obj, field } => {
                let obj_type = src_regs[obj.0 as usize];
                Opcode::Field {
                    dst,
                    obj,
                    field: self.remap_field(obj_type.0, field),
                }
            }
            Opcode::SetField { obj, field, src } => {
                let obj_type = src_regs[obj.0 as usize];
                Opcode::SetField {
                    obj,
                    field: self.remap_field(obj_type.0, field),
                    src,
                }
            }
            Opcode::GetThis { dst, field } => {
                // 'this' is always register 0
                let this_type = src_regs[0];
                Opcode::GetThis {
                    dst,
                    field: self.remap_field(this_type.0, field),
                }
            }
            Opcode::SetThis { field, src } => {
                // 'this' is always register 0
                let this_type = src_regs[0];
                Opcode::SetThis {
                    field: self.remap_field(this_type.0, field),
                    src,
                }
            }

            // Enum opcodes - need to remap construct indices based on enum type
            Opcode::MakeEnum {
                dst,
                construct,
                args,
            } => {
                // dst register type tells us the enum type
                let enum_type = src_regs[dst.0 as usize];
                Opcode::MakeEnum {
                    dst,
                    construct: self.remap_enum_construct_for_type(enum_type.0, construct),
                    args,
                }
            }
            Opcode::EnumAlloc { dst, construct } => {
                // dst register type tells us the enum type
                let enum_type = src_regs[dst.0 as usize];
                Opcode::EnumAlloc {
                    dst,
                    construct: self.remap_enum_construct_for_type(enum_type.0, construct),
                }
            }
            Opcode::EnumField {
                dst,
                value,
                construct,
                field,
            } => {
                // value register type tells us the enum type
                let enum_type = src_regs[value.0 as usize];
                Opcode::EnumField {
                    dst,
                    value,
                    construct: self.remap_enum_construct_for_type(enum_type.0, construct),
                    field,
                }
            }
            Opcode::SetEnumField { value, field, src } => {
                // SetEnumField doesn't have construct, but we don't need to remap construct here
                // The field index is relative to the construct, which is determined at runtime
                // Just pass through after applying standard remapping
                Opcode::SetEnumField { value, field, src }
            }

            // Switch opcode - remap offsets based on enum construct index remapping
            Opcode::Switch { reg, offsets, end } => {
                // Check if the switch register is an enum type that needs construct remapping
                let enum_type = src_regs[reg.0 as usize];

                if let Some(construct_map) = self.enum_type_constructs.get(&enum_type.0) {
                    // Get target construct count (or fall back to computing from map)
                    let target_count = self
                        .enum_type_target_counts
                        .get(&enum_type.0)
                        .copied()
                        .unwrap_or_else(|| {
                            // Compute from max target index + 1
                            construct_map.values().max().map(|&m| m + 1).unwrap_or(offsets.len())
                        });

                    // Create new offsets vector sized for target enum
                    // Initialize all entries to 'end' (default case)
                    let mut new_offsets = vec![end; target_count];

                    // Remap: new_offsets[target_idx] = old_offsets[src_idx]
                    for (&src_idx, &target_idx) in construct_map {
                        if src_idx < offsets.len() && target_idx < new_offsets.len() {
                            new_offsets[target_idx] = offsets[src_idx];
                        }
                    }

                    // For source indices that weren't remapped (same index in both),
                    // copy them directly if within bounds
                    for (src_idx, &offset) in offsets.iter().enumerate() {
                        if !construct_map.contains_key(&src_idx) && src_idx < new_offsets.len() {
                            new_offsets[src_idx] = offset;
                        }
                    }

                    Opcode::Switch {
                        reg,
                        offsets: new_offsets,
                        end,
                    }
                } else {
                    // No remapping needed
                    Opcode::Switch { reg, offsets, end }
                }
            }

            // Delegate all other opcodes to the standard remap_opcode
            other => self.remap_opcode(&other),
        }
    }
}
