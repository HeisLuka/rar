mod journal;
mod layout;
mod model;
mod recovery;
mod staging;
mod transaction;

pub use journal::JournalStore;
pub use layout::{InstallLayout, MutationLock, SpaceBudget, tree_sha256};
pub use model::{UpdateJournal, UpdateMode, UpdatePhase};
pub use recovery::{RecoveryDirective, TreeIdentity, reconcile};

pub use transaction::{
    Failpoint, ProductSelfCheck, SelfCheckResult, TransactionOutcome, UpdateTransaction,
    prepare_payload_swap, resume_transaction,
};

pub use staging::{
    ProductTreeManifest, StagedTree, TREE_MANIFEST_SCHEMA_V1, TreeFile, stage_zip_payload,
};
