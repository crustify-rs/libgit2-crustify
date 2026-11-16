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
    __test::index::api::ReucEntry,
    __test::index::api::NameEntry,
    __test::repository::api::RepositoryInitMode,
    __test::repository::api::RefStorage,
    RepositoryInitOptions,
    RepositoryState,
    __test::repository::api::RepositoryItem,
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
    __test::config::ConfigMap<'static>,
    Stage,
    IndexTime,
    IndexEntry,
    __test::index::IndexCapability,
    __test::index::IndexAddOption,
    __test::index::ConflictEntries,
    __test::commit::api::SignatureRef<'static>,
    CommitBuilder,
    __test::tree::api::EntryKind,
    __test::tree::api::TreeWalkMode,
    __test::tree::api::TreeWalk,
    __test::tree::api::TreeUpdate,
    Sort,
    Reference<'static>,
    __test::refs_api::PeelTarget,
    __test::refs_api::ReferenceFormat,
    Signature,
    Reflog,
    __test::refs_api::RefValue,
    HashSigOptions,
    HashSig,
    OidType,
    Oid,
    __test::oid::HexOid,
    OidShortener,
    __test::oidarray::OidArray,
    __test::strarray::StrArray,
    Libgit2,
    ErrorCode,
    ErrorClass,
    Error,
);

send_not_sync!(
    Backend,
    References<'static>,
    __test::refs_api::ReferenceNames<'static>,
);

neither!(
    __test::repository::Refdb<'static>,
    OdbWriter<'static>,
    OdbReader<'static>,
    Index<'static>,
    __test::index::IndexEntries<'static>,
    __test::index::IndexConflicts<'static>,
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
