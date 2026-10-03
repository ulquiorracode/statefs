//! Standard Terminal Adapter for any StateFS `Store`.
//!
//! Bridges `statefs-core` storage to the bottom of the Sewing Machine U-cycle.

use crate::intent::{Refusal, StateIntent, StateOutcome};
use crate::layer::Terminal;
use statefs_core::{Node, Store};

/// Terminal adapter that applies `StateIntent` directly to a mutable `Store`.
pub struct StoreTerminal;

impl<S: Store> Terminal<S, StateIntent, StateOutcome> for StoreTerminal {
    #[inline]
    fn execute(&mut self, store: &mut S, intent: StateIntent) -> Result<StateOutcome, Refusal> {
        match intent {
            StateIntent::Get { path } => {
                let node = store.get(&path).cloned();
                Ok(StateOutcome::Read(node))
            }
            StateIntent::Set { path, value } => {
                let node = Node::new(value);
                let previous = store.insert_node(&path, node.clone())?;
                Ok(StateOutcome::Written {
                    path,
                    previous,
                    current: node,
                })
            }
            StateIntent::SetNode { path, node } => {
                let previous = store.insert_node(&path, node.clone())?;
                Ok(StateOutcome::Written {
                    path,
                    previous,
                    current: node,
                })
            }
            StateIntent::Remove { path } => {
                let removed = store.remove(&path)?;
                Ok(StateOutcome::Removed { path, removed })
            }
            StateIntent::ListChildren { prefix } => {
                let paths = store.list_children(&prefix);
                Ok(StateOutcome::PathList(paths))
            }
            StateIntent::ListSubpaths { prefix } => {
                let paths = store.list_subpaths(&prefix);
                Ok(StateOutcome::PathList(paths))
            }
        }
    }
}
