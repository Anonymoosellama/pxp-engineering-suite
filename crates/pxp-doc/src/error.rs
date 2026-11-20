// SPDX-License-Identifier: LGPL-2.1-or-later
//! The document error type.

use crate::ObjectId;
use crate::expr::ExprError;
use std::fmt;

/// Everything that can go wrong in a document operation. Library code never
/// panics; the application shell turns these into user messages (and an
/// [`pxp_diag::ErrorReport`] behind "copy report").
#[derive(Debug, Clone, PartialEq)]
pub enum DocError {
    /// No object has this ID (it may have been deleted).
    NotFound(ObjectId),
    /// Deleting `id` would leave `dependants` with a dangling reference. Use
    /// [`crate::Tx::delete_with_dependants`] to delete them too.
    HasDependants {
        /// The object that could not be deleted.
        id: ObjectId,
        /// Objects that still depend on it.
        dependants: Vec<ObjectId>,
    },
    /// The edit would make `from` depend (directly or transitively) on
    /// itself through `to`.
    Cycle {
        /// The object whose dependencies were being set.
        from: ObjectId,
        /// The dependency that closes the cycle.
        to: ObjectId,
    },
    /// A dependency names an object that does not exist.
    DanglingDependency {
        /// The object holding the dependency.
        from: ObjectId,
        /// The missing target.
        to: ObjectId,
    },
    /// Another expression already has this name.
    DuplicateExpressionName(String),
    /// Not a valid name (expression, application id or object kind).
    InvalidName(String),
    /// Layer outside `1..=256`.
    InvalidLayer(u16),
    /// The named expression's formula failed to parse or evaluate.
    Expression {
        /// Expression name (or the formula, for anonymous evaluation).
        name: String,
        /// What went wrong.
        error: ExprError,
    },
    /// The object exists but is not an expression.
    NotAnExpression(ObjectId),
    /// A command panicked inside a transaction. The transaction was rolled
    /// back completely and the session continues (master prompt:
    /// "Panics are caught at the command boundary").
    Panicked {
        /// The transaction label (command name).
        label: String,
        /// The panic message.
        message: String,
    },
    /// A long operation was cancelled; nothing was committed.
    Cancelled,
    /// Undo was requested with an empty undo history.
    NothingToUndo,
    /// Redo was requested with an empty redo history.
    NothingToRedo,
}

impl fmt::Display for DocError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound(id) => write!(f, "object {id} not found"),
            Self::HasDependants { id, dependants } => write!(
                f,
                "object {id} cannot be deleted: {} object(s) depend on it",
                dependants.len()
            ),
            Self::Cycle { from, to } => {
                write!(f, "dependency of {from} on {to} would create a cycle")
            }
            Self::DanglingDependency { from, to } => {
                write!(f, "object {from} depends on missing object {to}")
            }
            Self::DuplicateExpressionName(n) => {
                write!(f, "an expression named '{n}' already exists")
            }
            Self::InvalidName(n) => write!(f, "'{n}' is not a valid name"),
            Self::InvalidLayer(n) => write!(f, "layer {n} is outside 1..=256"),
            Self::Expression { name, error } => write!(f, "expression '{name}': {error}"),
            Self::NotAnExpression(id) => write!(f, "object {id} is not an expression"),
            Self::Panicked { label, message } => {
                write!(
                    f,
                    "'{label}' failed unexpectedly and was rolled back: {message}"
                )
            }
            Self::Cancelled => write!(f, "cancelled; nothing was changed"),
            Self::NothingToUndo => write!(f, "nothing to undo"),
            Self::NothingToRedo => write!(f, "nothing to redo"),
        }
    }
}

impl std::error::Error for DocError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Expression { error, .. } => Some(error),
            _ => None,
        }
    }
}
