use hlbc::types::{Reg, RefType};

use crate::ast::{Constant, Expr, Statement};

#[derive(Debug)]
pub(crate) enum ScopeType {
    Len(i32),
    Manual,
}

#[derive(Debug)]
pub(crate) enum ScopeData {
    Root,
    If {
        cond: Expr,
    },
    Else {
        if_cond: Expr,
        if_stmts: Vec<Statement>,
    },
    Switch {
        arg: Expr,
        offsets: Vec<usize>,
        cases: Vec<(Vec<usize>, Vec<Statement>)>,
        /// If switching on an EnumIndex, this holds the enum type for constructor lookup
        enum_type: Option<RefType>,
    },
    SwitchCase {
        /// Multiple patterns for combined cases (e.g., case 0, 1, 2:)
        patterns: Vec<usize>,
    },
    Loop {
        start: usize,
        cond: Expr,
    },
    /// Try scope - opened by OTrap, closed by OEndTrap
    Try {
        /// Exception register from OTrap (will hold caught exception)
        exc: Reg,
        /// Absolute opcode index where catch block starts
        catch_start: usize,
    },
    /// Catch scope - opened when we reach catch_start address
    Catch {
        /// Exception register (same as Try's exc) - stored for debugging
        #[allow(dead_code)]
        exc: Reg,
        /// Statements from the closed Try scope
        try_stmts: Vec<Statement>,
    },
}

#[derive(Debug)]
pub(crate) struct Scope {
    pub(crate) ty: ScopeType,
    pub(crate) stmts: Vec<Statement>,
    pub(crate) data: ScopeData,
}

impl Scope {
    fn new(ty: ScopeType, data: ScopeData) -> Self {
        Self {
            ty,
            stmts: Vec::new(),
            data,
        }
    }

    /// Finish the scope by creating a statement from it
    pub(crate) fn make_stmt(self) -> Statement {
        match self.data {
            ScopeData::If { cond } => Statement::IfElse {
                cond,
                if_: self.stmts,
                else_: Vec::new(),
            },
            ScopeData::Else { if_cond, if_stmts } => Statement::IfElse {
                cond: if_cond,
                if_: if_stmts,
                else_: self.stmts,
            },
            ScopeData::Switch { arg, cases, enum_type, .. } => Statement::Switch {
                arg,
                default: self.stmts,
                cases,
                enum_type,
            },
            ScopeData::Loop { cond, .. } => Statement::While {
                cond,
                stmts: self.stmts,
            },
            ScopeData::Try { .. } => {
                // Try scope should be closed via close_try(), not make_stmt()
                // If we get here, emit try block as a comment (unclosed try)
                Statement::Comment(format!("unclosed try block with {} statements", self.stmts.len()))
            }
            ScopeData::Catch { try_stmts, .. } => Statement::TryCatch {
                try_stmts,
                catch_var: "e".to_string(),
                catch_stmts: self.stmts,
            },
            ScopeData::Root => {
                // Root scope shouldn't be converted to statement - wrap in a block
                Statement::Block { stmts: self.stmts }
            }
            ScopeData::SwitchCase { patterns } => {
                // SwitchCase should be merged into parent Switch, not standalone
                Statement::Comment(format!("orphan switch case: {:?}", patterns))
            }
        }
    }
}

/// Helper to process a stack of scopes (branches, loops)
pub(crate) struct Scopes {
    /// There is always at least one scope, the root scope
    pub(crate) scopes: Vec<Scope>,
}

impl Scopes {
    pub(crate) fn new() -> Self {
        Self {
            scopes: vec![Scope::new(ScopeType::Manual, ScopeData::Root)],
        }
    }

    pub(crate) fn push_stmt(&mut self, stmt: Statement) {
        self.scopes.last_mut().unwrap().stmts.push(stmt);
    }

    /// Advance the scope counters and close any scopes that have reached their end.
    /// Returns (if_else_closed, switches_closed) for register state restoration.
    pub(crate) fn advance(&mut self) -> (usize, usize) {
        let mut stmt = None;
        let mut if_else_closed = 0;
        let mut switches_closed = 0;

        // Phase 1: Identify scopes that need to close (len == 1)
        // and decrement others, but don't modify the vector yet
        let mut to_close: Vec<usize> = Vec::new();
        for i in (0..self.scopes.len()).rev() {
            if matches!(self.scopes[i].ty, ScopeType::Len(len) if len == 1) {
                to_close.push(i);
            } else if let ScopeType::Len(ref mut len) = self.scopes[i].ty {
                *len -= 1;
            }
        }

        // Phase 2: Process closures from highest index to lowest (safe removal order)
        // Note: to_close is already in descending order from the rev() iteration
        for i in to_close {
            // Safety check: ensure index is still valid after previous removals
            if i >= self.scopes.len() {
                continue;
            }

            let mut scope = self.scopes.remove(i);

            // Track If/Else closures for register state restoration
            if matches!(scope.data, ScopeData::If { .. } | ScopeData::Else { .. }) {
                if_else_closed += 1;
            }

            // Exception for Switch: close any remaining nested scopes and SwitchCase
            // After removing the Switch at i, all scopes that were above it are now at index >= i
            // We need to close them and collect the final SwitchCase
            if let ScopeData::Switch { cases, .. } = &mut scope.data {
                switches_closed += 1;
                // Close all scopes above where Switch was, from highest index to lowest
                while self.scopes.len() > i {
                    let inner_scope = self.scopes.pop().unwrap();
                    match inner_scope.data {
                        ScopeData::SwitchCase { patterns } => {
                            // Found the last case - add it to the switch's cases
                            cases.push((patterns, inner_scope.stmts));
                        }
                        ScopeData::If { .. } | ScopeData::Else { .. } => {
                            // Track nested If/Else closures too
                            if_else_closed += 1;
                            let inner_stmt = inner_scope.make_stmt();
                            if self.scopes.len() > i {
                                self.scopes.last_mut().unwrap().stmts.push(inner_stmt);
                            } else {
                                scope.stmts.push(inner_stmt);
                            }
                        }
                        _ => {
                            // It's a nested scope - convert to statement
                            // and add to the scope below (which might be the SwitchCase)
                            let inner_stmt = inner_scope.make_stmt();
                            if self.scopes.len() > i {
                                self.scopes.last_mut().unwrap().stmts.push(inner_stmt);
                            } else {
                                // No more scopes above - add to switch's default
                                scope.stmts.push(inner_stmt);
                            }
                        }
                    }
                }
            }

            let closed_stmt = scope.make_stmt();

            // Immediately push the closed statement to the current top scope if one exists,
            // rather than carrying it forward. This ensures nested switch statements go to
            // the correct SwitchCase instead of the parent switch's default.
            if let Some(current_scope) = self.scopes.last_mut() {
                current_scope.stmts.push(closed_stmt);
            } else {
                stmt = Some(closed_stmt);
            }
        }

        // Phase 3: Push any remaining statement to the current scope (only if no scopes were left)
        if let Some(s) = stmt {
            if let Some(scope) = self.scopes.last_mut() {
                scope.stmts.push(s);
            }
        }

        (if_else_closed, switches_closed)
    }

    pub(crate) fn statements(mut self) -> Vec<Statement> {
        // Gracefully handle unclosed scopes by folding them into the result
        let mut result = Vec::new();

        // If there are remaining scopes beyond root, try to close them gracefully
        while self.scopes.len() > 1 {
            let scope = self.scopes.pop().unwrap();
            // Add a comment about the unclosed scope
            result.push(Statement::Comment(format!(
                "unclosed scope: {:?}",
                std::mem::discriminant(&scope.data)
            )));
            // Try to include statements from the unclosed scope
            result.extend(scope.stmts);
        }

        // Now get the root scope
        if let Some(Scope { stmts, data, .. }) = self.scopes.pop() {
            if matches!(data, ScopeData::Root) {
                // Prepend root statements, then add any from unclosed scopes
                let mut final_result = stmts;
                final_result.extend(result);
                final_result
            } else {
                // Even the last scope isn't Root - unusual but handle it
                result.push(Statement::Comment("unexpected final scope (not Root)".to_string()));
                result.extend(stmts);
                result
            }
        } else {
            // No scopes at all - return empty with a comment
            vec![Statement::Comment("no scopes found".to_string())]
        }
    }

    pub(crate) fn push_if(&mut self, len: i32, cond: Expr) {
        self.scopes
            .push(Scope::new(ScopeType::Len(len), ScopeData::If { cond }))
    }

    pub(crate) fn push_else(&mut self, len: i32) {
        // Try to find the matching If scope
        let if_data = self
            .scopes
            .pop()
            .and_then(|s| match s.data {
                ScopeData::If { cond } => Some((cond, s.stmts)),
                _ => {
                    // Not an If - put it back
                    self.scopes.push(s);
                    None
                }
            });

        if let Some((if_cond, stmts)) = if_data {
            self.scopes.push(Scope::new(
                ScopeType::Len(len),
                ScopeData::Else {
                    if_cond,
                    if_stmts: stmts,
                },
            ));
        } else {
            // No matching If - emit a comment instead
            self.last_mut().stmts.push(Statement::Comment(format!(
                "else block (len={}) without matching if",
                len
            )));
        }
    }

    pub(crate) fn push_switch(&mut self, len: i32, arg: Expr, offsets: Vec<usize>, enum_type: Option<RefType>) {
        self.scopes.push(Scope::new(
            ScopeType::Len(len),
            ScopeData::Switch {
                arg,
                offsets,
                cases: Vec::new(),
                enum_type,
            },
        ))
    }

    /// Push a switch case with potentially multiple patterns (for combined cases like `case 0, 1, 2:`)
    pub(crate) fn push_switch_case(&mut self, patterns: Vec<usize>) {
        // Find the Switch scope in the stack
        let switch_idx = self.scopes.iter().rposition(|s| {
            matches!(s.data, ScopeData::Switch { .. })
        });

        let Some(switch_idx) = switch_idx else {
            // No switch context found - emit a comment
            self.last_mut().stmts.push(Statement::Comment(format!(
                "switch case {:?} (no outer switch context)",
                patterns
            )));
            return;
        };

        // Close all scopes above the switch (nested loops, ifs, etc.) and merge into current case
        // These are scopes that were opened inside the previous case and haven't closed yet
        while self.scopes.len() > switch_idx + 1 {
            let inner_scope = self.scopes.pop().unwrap();
            match inner_scope.data {
                ScopeData::SwitchCase { patterns } => {
                    // Found the previous case - add it to the switch's cases
                    if let ScopeData::Switch { cases, .. } = &mut self.scopes[switch_idx].data {
                        cases.push((patterns, inner_scope.stmts));
                    }
                }
                _ => {
                    // It's a nested scope (loop, if, etc.) - convert to statement
                    // and add to whatever is now on top
                    let stmt = inner_scope.make_stmt();
                    if self.scopes.len() > switch_idx + 1 {
                        // Add to the scope below (which might be the SwitchCase)
                        self.scopes.last_mut().unwrap().stmts.push(stmt);
                    } else {
                        // We're at the switch level - this shouldn't happen often,
                        // but if it does, add to default case
                        self.scopes[switch_idx].stmts.push(stmt);
                    }
                }
            }
        }

        // Now push the new SwitchCase
        self.scopes.push(Scope::new(
            ScopeType::Manual,
            ScopeData::SwitchCase { patterns },
        ));
    }

    pub(crate) fn push_loop(&mut self, start: usize) {
        self.scopes.push(Scope::new(
            ScopeType::Manual,
            ScopeData::Loop {
                start,
                cond: Expr::Constant(Constant::Bool(true)),
            },
        ))
    }

    /// Push a try scope. Closed explicitly by OEndTrap handler.
    pub(crate) fn push_try(&mut self, exc: Reg, catch_start: usize) {
        self.scopes.push(Scope::new(
            ScopeType::Manual, // Don't use Len - we close explicitly on OEndTrap
            ScopeData::Try { exc, catch_start },
        ))
    }

    /// Push a catch scope. Uses Len-based scope type since catch length is known.
    pub(crate) fn push_catch(&mut self, exc: Reg, len: i32, try_stmts: Vec<Statement>) {
        self.scopes.push(Scope::new(
            ScopeType::Len(len),
            ScopeData::Catch { exc, try_stmts },
        ))
    }

    /// Find and close the innermost Try scope, returning its data.
    /// Also closes any nested scopes inside the try.
    pub(crate) fn close_try(&mut self) -> Option<(Reg, usize, Vec<Statement>)> {
        // Find the innermost Try scope
        let try_idx = self.scopes.iter().rposition(|s| {
            matches!(s.data, ScopeData::Try { .. })
        })?;

        // Close any scopes nested inside the try
        while self.scopes.len() > try_idx + 1 {
            let inner = self.scopes.pop().unwrap();
            let stmt = inner.make_stmt();
            self.scopes[try_idx].stmts.push(stmt);
        }

        // Close the try scope itself
        let try_scope = self.scopes.pop().unwrap();
        if let ScopeData::Try { exc, catch_start } = try_scope.data {
            Some((exc, catch_start, try_scope.stmts))
        } else {
            None
        }
    }

    //region QUERIES
    /// Returns a mutable reference to the loop condition if the current scope is a loop
    pub(crate) fn last_loop_cond_mut(&mut self) -> Option<&mut Expr> {
        self.scopes.last_mut().and_then(|s| match &mut s.data {
            ScopeData::Loop { cond, .. } => Some(cond),
            _ => None,
        })
    }

    /// Returns the start index of the last loop in the scope stack
    pub(crate) fn last_loop_start(&self) -> Option<usize> {
        self.scopes.iter().rev().find_map(|s| match s.data {
            ScopeData::Loop { start, .. } => Some(start),
            _ => None,
        })
    }

    /// End the innermost loop scope, closing any nested scopes inside it first.
    /// This handles cases where there are If/Switch scopes inside the loop that
    /// haven't been closed yet when we hit the backward JAlways.
    pub(crate) fn end_last_loop(&mut self) -> Option<Statement> {
        // Find the innermost loop scope
        let loop_idx = self
            .scopes
            .iter()
            .rposition(|s| matches!(s.data, ScopeData::Loop { .. }))?;

        // Close all scopes that are inside the loop (on top of it in the stack)
        // and collect them into the loop's body
        while self.scopes.len() > loop_idx + 1 {
            let inner_scope = self.scopes.pop().unwrap();

            match inner_scope.data {
                ScopeData::SwitchCase { patterns } => {
                    // Find the parent Switch scope and merge this case into it
                    let switch_idx = self.scopes[loop_idx..].iter().rposition(|s| {
                        matches!(s.data, ScopeData::Switch { .. })
                    }).map(|i| loop_idx + i);

                    if let Some(switch_idx) = switch_idx {
                        if let ScopeData::Switch { cases, .. } = &mut self.scopes[switch_idx].data {
                            cases.push((patterns, inner_scope.stmts));
                            continue;
                        }
                    }
                    // No Switch found - emit as orphan comment
                    self.scopes[loop_idx].stmts.push(Statement::Comment(
                        format!("orphan switch case in loop: {:?}", patterns)
                    ));
                }
                _ => {
                    // Regular scope - convert to statement
                    let inner_stmt = inner_scope.make_stmt();
                    // Add to the scope that's now on top (might be another nested scope)
                    if self.scopes.len() > loop_idx + 1 {
                        self.scopes.last_mut().unwrap().stmts.push(inner_stmt);
                    } else {
                        self.scopes[loop_idx].stmts.push(inner_stmt);
                    }
                }
            }
        }

        // Now the loop is at the top, pop and return it
        self.scopes.pop().map(|s| s.make_stmt())
    }

    /// Returns the switch jump offsets if we're currently inside a switch context
    /// (either directly in a Switch scope, in a SwitchCase, or in nested scopes inside a switch)
    pub(crate) fn last_is_switch_ctx(&self) -> Option<&[usize]> {
        // Search from innermost to outermost for a Switch context
        for scope in self.scopes.iter().rev() {
            match &scope.data {
                ScopeData::Switch { offsets, .. } => return Some(offsets.as_slice()),
                ScopeData::SwitchCase { .. } => {
                    // Found a SwitchCase, the Switch should be right below it
                    // Continue searching to find the parent Switch
                    continue;
                }
                _ => continue,
            }
        }
        None
    }

    pub(crate) fn last_is_if(&self) -> bool {
        self.scopes
            .last()
            .map(|s| matches!(&s.data, ScopeData::If { .. }))
            .unwrap_or(false)
    }

    pub(crate) fn has_scopes(&self) -> bool {
        self.scopes.len() > 1
    }

    /// Get a mutable reference to the last (innermost) scope
    pub(crate) fn last_mut(&mut self) -> &mut Scope {
        self.scopes.last_mut().expect("No scopes available")
    }
    //endregion
}
