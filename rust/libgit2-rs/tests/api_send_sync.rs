//! The thread-safety of every public type of the safe API, stated and
//! checked at compile time: `send_sync!` (the type is `Send` / `Sync`) and
//! `not_send!` / `not_sync!` (it is not: the check fails to compile if the
//! type gains the trait), so that a change of either is a deliberate edit
//! here.
//!
//! The rules (see each type's documentation): what libgit2 lets several
//! threads use at once is `Send + Sync` (`Repository`, `Odb`, `Config`,
//! `Reference`, ids, errors, options); single-threaded C objects (the index
//! and its iterators, objects and trees, revwalks, odb streams, reflog
//! entries, the refdb handle) are neither; iterators and backends that one
//! thread drives at a time are `Send` but not `Sync`.
#![forbid(unsafe_code)]
#![allow(dead_code)]

use libgit2_rs::*;

/// Fails to compile when `$t` implements `$tr` (two impls would apply).
macro_rules! assert_not_impl {
    ($t:ty, $tr:path) => {
        const _: fn() = || {
            trait AmbiguousIfImpl<A> {
                fn some_item() {}
            }
            impl<T: ?Sized> AmbiguousIfImpl<()> for T {}
            struct Invalid;
            impl<T: ?Sized + $tr> AmbiguousIfImpl<Invalid> for T {}
            let _ = <$t as AmbiguousIfImpl<_>>::some_item;
        };
    };
}

macro_rules! send_sync {
    ($($t:ty),* $(,)?) => {
        $(const _: fn() = || {
            fn send<T: Send>() {}
            fn sync<T: Sync>() {}
            send::<$t>();
            sync::<$t>();
        };)*
    };
}

macro_rules! send_not_sync {
    ($($t:ty),* $(,)?) => {
        $(const _: fn() = || {
            fn send<T: Send>() {}
            send::<$t>();
        };
        assert_not_impl!($t, Sync);)*
    };
}

macro_rules! neither {
    ($($t:ty),* $(,)?) => {
        $(assert_not_impl!($t, Send);
        assert_not_impl!($t, Sync);)*
    };
}

send_sync!(
    index::api::ReucEntry,
    index::api::NameEntry,
    repository::api::RepositoryInitMode,
    repository::api::RefStorage,
    RepositoryInitOptions,
    RepositoryState,
    repository::api::RepositoryItem,
    Repository,
    Odb<'static>,
    ExpandId,
    OdbObject,
    LooseOptions,
    Mempack,
    ConfigLevel,
    ConfigEntry,
    Config<'static>,
    ConfigEntries<'static>,
    config::ConfigMap<'static>,
    Stage,
    IndexTime,
    IndexEntry,
    index::IndexCapability,
    index::IndexAddOption,
    index::ConflictEntries,
    commit::api::SignatureRef<'static>,
    CommitBuilder,
    tree::api::EntryKind,
    tree::api::TreeWalkMode,
    tree::api::TreeWalk,
    tree::api::TreeUpdate,
    Sort,
    Reference<'static>,
    refs_api::PeelTarget,
    refs_api::ReferenceFormat,
    refs_api::Committer,
    Reflog,
    refs_api::RefValue,
    HashSigOptions,
    HashSig,
    OidType,
    Oid,
    oid::HexOid,
    OidShortener,
    OidArray,
    StrArray,
    Libgit2,
    ErrorCode,
    ErrorClass,
    Error,
    Url,
    ProcessOptions,
    ProcessStatus,
    Process,
    SortedCache<u32>,
);

send_not_sync!(
    Backend,
    References<'static>,
    refs_api::ReferenceNames<'static>,
);

neither!(
    repository::Refdb<'static>,
    OdbWriter<'static>,
    OdbReader<'static>,
    Index<'static>,
    index::IndexEntries<'static>,
    index::IndexConflicts<'static>,
    Commit<'static>,
    TreeEntryRef<'static>,
    TreeEntry,
    Tree<'static>,
    TreeBuilder<'static>,
    Revwalk<'static>,
    ReflogEntry,
);

#[test]
fn stated_at_compile_time() {
    // The assertions above are checked when this file compiles.
}
