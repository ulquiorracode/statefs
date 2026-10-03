//! # StateFS Stitch
//!
//! Monomorphic U-Cycle Stitching Pipeline Engine implementing
//! **The Sewing Machine Architecture (SMA)**.
//!
//! > **Result = Material (Fabric) + Intent (Thread) + Work (Needle & Machine)**
//!
//! - **Material**: The passive state and invariants (`TCtx` / `Store`).
//! - **Intent**: The desire entering the pipeline (`TIntent`).
//! - **Work**: The monomorphic pipeline (`Machine`) driving the U-cycle:
//!   - Phase 1: Descent (`admit`: Accept or Refuse)
//!   - Point of Puncture: Execution at `Terminal`
//!   - Phase 2: Ascent (`tighten`: Diffs, events, telemetry)

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

pub mod cqs;
pub mod intent;
pub mod layer;
pub mod machine;
pub mod store_terminal;

pub use cqs::{Command, Query};
pub use intent::{Admission, Refusal, StateIntent, StateOutcome};
pub use layer::{Layer, Middleware, Terminal, TerminalHandler};
pub use machine::{Machine, Pipeline, StackNode, StitchChain, TerminalNode};
pub use store_terminal::StoreTerminal;

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::String;
    use alloc::vec::Vec;
    use statefs_core::{MemStore, Path, Value};

    // Layer 1: Access Guard (Отказывает или Принимает)
    struct PathAccessGuard {
        blocked_prefix: Path,
    }

    impl<S> Layer<S, StateIntent, StateOutcome> for PathAccessGuard {
        fn admit(&self, _ctx: &mut S, intent: StateIntent) -> Admission<StateIntent> {
            match &intent {
                StateIntent::Set { path, .. } | StateIntent::Remove { path } => {
                    if path.starts_with(&self.blocked_prefix) {
                        Admission::Refuse(Refusal::AccessDenied("Path is protected by AccessGuard"))
                    } else {
                        Admission::Admit(intent)
                    }
                }
                _ => Admission::Admit(intent),
            }
        }

        fn tighten(&self, _ctx: &mut S, _outcome: &mut Result<StateOutcome, Refusal>) {
            // Guard doesn't need to do anything on ascent
        }
    }

    // Layer 2: Reactive Telemetry & Audit (Записывает стежки на подъеме)
    #[derive(Default)]
    struct AuditLogLayer {
        log: alloc::rc::Rc<core::cell::RefCell<Vec<String>>>,
    }

    impl<S> Layer<S, StateIntent, StateOutcome> for AuditLogLayer {
        fn admit(&self, _ctx: &mut S, intent: StateIntent) -> Admission<StateIntent> {
            Admission::Admit(intent)
        }

        fn tighten(&self, _ctx: &mut S, outcome: &mut Result<StateOutcome, Refusal>) {
            // На подъеме игла сообщает, что произошло
            match outcome {
                Ok(StateOutcome::Written { path, .. }) => {
                    self.log
                        .borrow_mut()
                        .push(alloc::format!("Written: {}", path));
                }
                Err(Refusal::AccessDenied(reason)) => {
                    self.log
                        .borrow_mut()
                        .push(alloc::format!("Refused: {}", reason));
                }
                _ => {}
            }
        }
    }

    #[test]
    fn test_sewing_machine_u_cycle() {
        let mut store = MemStore::new();

        let audit_log = alloc::rc::Rc::new(core::cell::RefCell::new(Vec::new()));

        // Строим швейную машинку:
        // Terminal (Дно: MemStore)
        //   -> оборачиваем в PathAccessGuard (внутренний защитный слой)
        //   -> оборачиваем в AuditLogLayer (внешний слой наблюдателя)
        let mut machine = Machine::on_terminal(StoreTerminal)
            .wrap(PathAccessGuard {
                blocked_prefix: Path::parse("/system/secure"),
            })
            .wrap(AuditLogLayer {
                log: alloc::rc::Rc::clone(&audit_log),
            });

        // Стежок 1: Разрешенная запись
        // Намерение: Установить motd сервера
        let motd_intent = StateIntent::Set {
            path: Path::parse("/server/motd"),
            value: Value::from("Welcome to StateFS!"),
        };

        // РАБОТА: Игла делает стежок
        let outcome = machine.stitch(&mut store, motd_intent).unwrap();
        assert!(matches!(outcome, StateOutcome::Written { .. }));

        // Проверяем, что в ткани (в ядре памяти) факт действительно зафиксирован
        let read_intent = StateIntent::Get {
            path: Path::parse("/server/motd"),
        };
        let read_outcome = machine.stitch(&mut store, read_intent).unwrap();
        match read_outcome {
            StateOutcome::Read(Some(node)) => {
                assert_eq!(node.value, Value::from("Welcome to StateFS!"));
            }
            _ => panic!("Expected node to be read"),
        }

        // Стежок 2: Защищенный путь (Отказ на спуске)
        // Намерение: Изменить системную защищенную ветку
        let hack_intent = StateIntent::Set {
            path: Path::parse("/system/secure/root_key"),
            value: Value::from("malicious"),
        };

        let err = machine.stitch(&mut store, hack_intent).unwrap_err();
        assert_eq!(
            err,
            Refusal::AccessDenied("Path is protected by AccessGuard")
        );

        // Убеждаемся, что ткань не была повреждена/изменена (игла не коснулась дна)
        let check_intent = StateIntent::Get {
            path: Path::parse("/system/secure/root_key"),
        };
        let check_outcome = machine.stitch(&mut store, check_intent).unwrap();
        assert_eq!(check_outcome, StateOutcome::Read(None));

        // 3. Проверяем, что на подъеме игла собрала правильные события в аудит-лог
        let events = audit_log.borrow();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0], "Written: /server/motd");
        assert_eq!(events[1], "Refused: Path is protected by AccessGuard");
    }
}
