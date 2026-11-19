//! The register file: X registers and float registers.

use crate::term::Term;

use super::{Process, ProcessError, X_REG_COUNT};

impl Process {
    /// Read X register `n`.
    #[must_use]
    pub fn x_reg(&self, n: u16) -> Term {
        self.x_regs[usize::from(n)]
    }

    /// Write X register `n`.
    pub fn set_x_reg(&mut self, n: u16, value: Term) {
        self.x_regs[usize::from(n)] = value;
    }

    /// Read float register `index`.
    pub fn get_float_reg(&self, index: u16) -> Result<f64, ProcessError> {
        self.float_regs
            .get(usize::from(index))
            .copied()
            .ok_or(ProcessError::InvalidFloatRegister { index })
    }

    /// Write float register `index`.
    pub fn set_float_reg(&mut self, index: u16, value: f64) -> Result<(), ProcessError> {
        let register = self
            .float_regs
            .get_mut(usize::from(index))
            .ok_or(ProcessError::InvalidFloatRegister { index })?;
        *register = value;
        Ok(())
    }

    /// Read all X registers.
    #[must_use]
    pub const fn x_regs(&self) -> &[Term; X_REG_COUNT] {
        &self.x_regs
    }

    /// Mutable access to all X registers.
    pub fn x_regs_mut(&mut self) -> &mut [Term; X_REG_COUNT] {
        &mut self.x_regs
    }

    /// Clear x registers at and above `live_x`.
    ///
    /// Called by minor GC after reclaiming the nursery: registers outside the
    /// traced live prefix may still point into reclaimed space, and a later
    /// full-register walk (major GC, conservative natives) must never chase
    /// them. The invariant is that every register always holds NIL, an
    /// immediate, or a pointer to a currently-allocated object.
    pub(crate) fn clear_dead_x_regs(&mut self, live_x: usize) {
        for reg in self.x_regs.iter_mut().skip(live_x) {
            *reg = Term::NIL;
        }
    }
}
