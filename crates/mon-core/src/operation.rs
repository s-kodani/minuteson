//! One directional reservation policy for every operation. `Wait` reserves its
//! resource before draining existing owners; it does not grant execution yet.
//! Recording may cancel preparation, whereas importing cannot preempt it.
use std::sync::Arc;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Worker,
    Recording,
    Import,
    ModelPreparation,
    GlobalMaintenance,
    ModelDeletion,
    MeetingDeletion,
    Export,
    Notice,
    Shutdown,
    Restart,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope<'a> {
    Global,
    Meeting(&'a str),
    ModelCatalog,
    Unscoped,
}
#[derive(Clone, Copy, Debug)]
pub struct Active<'a> {
    pub kind: Kind,
    pub scope: Scope<'a>,
}
#[derive(Clone, Copy, Debug)]
pub struct Request<'a> {
    pub kind: Kind,
    pub scope: Scope<'a>,
    pub resume_after_restart: bool,
    pub drain_notices: bool,
}
impl<'a> Request<'a> {
    pub fn new(kind: Kind, scope: Scope<'a>) -> Self {
        Self {
            kind,
            scope,
            resume_after_restart: false,
            drain_notices: false,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Effect {
    Allow,
    Wait,
    CancelAndWait,
    Block,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Decision {
    pub effect: Effect,
    pub owner: Option<Kind>,
}
impl Decision {
    pub fn can_reserve(self) -> bool {
        self.effect != Effect::Block
    }
    pub fn blocker(self) -> Option<Kind> {
        if self.effect == Effect::Block {
            self.owner
        } else {
            None
        }
    }
}
fn same_meeting(a: Scope<'_>, b: Scope<'_>) -> bool {
    matches!((a, b), (Scope::Meeting(a), Scope::Meeting(b)) if a == b)
}
pub fn relation(request: Request<'_>, active: Active<'_>) -> Effect {
    use Effect::*;
    use Kind::*;
    if active.kind == Shutdown {
        return Block;
    }
    if active.kind == Restart {
        return if request.kind == Shutdown
            || (request.resume_after_restart
                && matches!(request.kind, GlobalMaintenance | ModelDeletion))
        {
            Allow
        } else {
            Block
        };
    }
    match request.kind {
        Worker => {
            if matches!(
                active.kind,
                Recording | Import | ModelPreparation | GlobalMaintenance | ModelDeletion
            ) {
                Block
            } else {
                Allow
            }
        }
        Recording => match active.kind {
            Recording | Import | GlobalMaintenance | ModelDeletion => Block,
            ModelPreparation => CancelAndWait,
            Worker | MeetingDeletion => Wait,
            _ => Allow,
        },
        Import => match active.kind {
            Recording | Import | ModelPreparation | GlobalMaintenance | ModelDeletion => Block,
            Worker | MeetingDeletion => Wait,
            _ => Allow,
        },
        ModelPreparation => match active.kind {
            Recording | Import | ModelPreparation | GlobalMaintenance | ModelDeletion
            | MeetingDeletion => Block,
            Worker => Wait,
            _ => Allow,
        },
        GlobalMaintenance => {
            if active.kind == Notice && request.drain_notices {
                Wait
            } else {
                Block
            }
        }
        ModelDeletion => {
            if active.kind == Notice {
                Allow
            } else {
                Block
            }
        }
        MeetingDeletion => match active.kind {
            GlobalMaintenance | ModelPreparation | ModelDeletion | MeetingDeletion => Block,
            Notice if same_meeting(request.scope, active.scope) => Wait,
            _ => Allow,
        },
        Export => {
            if matches!(active.kind, GlobalMaintenance | ModelDeletion | Export) {
                Block
            } else {
                Allow
            }
        }
        Notice => match active.kind {
            GlobalMaintenance => Block,
            MeetingDeletion if same_meeting(request.scope, active.scope) => Block,
            _ => Allow,
        },
        Shutdown => {
            if matches!(active.kind, Recording | Export) {
                Block
            } else {
                CancelAndWait
            }
        }
        Restart => Block,
    }
}
pub fn decide(request: Request<'_>, active: &[Active<'_>]) -> Decision {
    let mut result = Decision {
        effect: Effect::Allow,
        owner: None,
    };
    for owner in active {
        let effect = relation(request, *owner);
        if effect > result.effect {
            result = Decision {
                effect,
                owner: Some(owner.kind),
            };
        }
    }
    result
}
pub fn same_owner<T>(a: &Arc<T>, b: &Arc<T>) -> bool {
    Arc::ptr_eq(a, b)
}
pub fn release_owned<T>(slot: &mut Option<Arc<T>>, owner: &Arc<T>) -> bool {
    if slot
        .as_ref()
        .is_some_and(|active| same_owner(active, owner))
    {
        *slot = None;
        true
    } else {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ALL: [Kind; 11] = [
        Kind::Worker,
        Kind::Recording,
        Kind::Import,
        Kind::ModelPreparation,
        Kind::GlobalMaintenance,
        Kind::ModelDeletion,
        Kind::MeetingDeletion,
        Kind::Export,
        Kind::Notice,
        Kind::Shutdown,
        Kind::Restart,
    ];
    #[test]
    fn every_pair_has_a_defined_decision_and_shutdown_fences_all_new_work() {
        // Columns follow ALL. A=allow, W=reserve then wait, C=cancel then
        // wait, B=reject before reservation. These are product rules, including
        // asymmetric recording/preparation and scoped deletion coexistence.
        let expected = [
            "ABBBBBAAABB",
            "WBBCBBWAABB",
            "WBBBBBWAABB",
            "WBBBBBBAABB",
            "BBBBBBBBBBB",
            "BBBBBBBBABB",
            "AAABBBBAABB",
            "AAAABBABABB",
            "AAAABAAAABB",
            "CBCCCCCBCBA",
            "BBBBBBBBBBB",
        ];
        for (row, request) in ALL.into_iter().enumerate() {
            assert_eq!(expected[row].len(), ALL.len());
            for (column, owner) in ALL.into_iter().enumerate() {
                let decision = decide(
                    Request::new(request, Scope::Global),
                    &[Active {
                        kind: owner,
                        scope: Scope::Global,
                    }],
                );
                let effect = match expected[row].as_bytes()[column] {
                    b'A' => Effect::Allow,
                    b'W' => Effect::Wait,
                    b'C' => Effect::CancelAndWait,
                    b'B' => Effect::Block,
                    _ => unreachable!(),
                };
                assert_eq!(
                    decision.effect, effect,
                    "request={request:?}, owner={owner:?}"
                );
            }
        }
    }
    #[test]
    fn scoped_notice_io_preserves_intended_model_and_other_meeting_concurrency() {
        let notice = Request::new(Kind::Notice, Scope::Meeting("a"));
        assert_eq!(
            relation(
                notice,
                Active {
                    kind: Kind::ModelDeletion,
                    scope: Scope::ModelCatalog
                }
            ),
            Effect::Allow
        );
        assert_eq!(
            relation(
                notice,
                Active {
                    kind: Kind::MeetingDeletion,
                    scope: Scope::Meeting("b")
                }
            ),
            Effect::Allow
        );
        assert_eq!(
            relation(
                notice,
                Active {
                    kind: Kind::MeetingDeletion,
                    scope: Scope::Meeting("a")
                }
            ),
            Effect::Block
        );
        let mut maintenance = Request::new(Kind::GlobalMaintenance, Scope::Global);
        maintenance.drain_notices = true;
        assert_eq!(
            relation(
                maintenance,
                Active {
                    kind: Kind::Notice,
                    scope: Scope::Meeting("a")
                }
            ),
            Effect::Wait
        );
    }
    #[test]
    fn recording_preempts_preparation_but_import_cannot_and_restart_resume_is_narrow() {
        let active = Active {
            kind: Kind::ModelPreparation,
            scope: Scope::ModelCatalog,
        };
        assert_eq!(
            relation(Request::new(Kind::Recording, Scope::Global), active),
            Effect::CancelAndWait
        );
        assert_eq!(
            relation(Request::new(Kind::Import, Scope::Global), active),
            Effect::Block
        );
        for kind in ALL {
            let mut request = Request::new(kind, Scope::Global);
            request.resume_after_restart = true;
            let allowed = relation(
                request,
                Active {
                    kind: Kind::Restart,
                    scope: Scope::Global,
                },
            ) == Effect::Allow;
            assert_eq!(
                allowed,
                matches!(
                    kind,
                    Kind::GlobalMaintenance | Kind::ModelDeletion | Kind::Shutdown
                )
            );
        }
    }
    #[test]
    fn a_cancelled_or_stale_lease_cannot_release_a_new_owner() {
        let old = Arc::new(());
        let new = Arc::new(());
        let mut held = Some(new.clone());
        assert!(!release_owned(&mut held, &old));
        assert!(held.is_some());
        assert!(release_owned(&mut held, &new));
        assert!(held.is_none());
        assert!(!release_owned(&mut held, &new));
    }
}
