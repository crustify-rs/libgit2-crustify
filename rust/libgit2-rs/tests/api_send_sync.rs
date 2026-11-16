//! The thread-safety of every public type of the safe API, stated and
//! checked at compile time: `send_sync!` (the type is `Send` / `Sync`) and
//! `not_send!` / `not_sync!` (it is not: the check fails to compile if the
//! type gains the trait), so that a change of either is a deliberate edit
//! here.
//!
//! The rules (see each type's documentation): what libgit2 lets several
//! threads use at once is `Send + Sync` (`Repository`, `Odb`, `Config`,
//! `Reference`, ids, errors, options); objects are immutable after parse
//! and counted atomically, so they are too (I-14); the index is
//! single-threaded in C, and the safe `Index` takes its lock for every
//! operation, so it is `Send + Sync` as well (I-1); iterators and backends
//! that one thread drives at a time are `Send` but not `Sync`; streams over
//! raw tables, revwalks, reflog entries and the refdb handle are neither.
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
    // errors, ids, options, plain data
    Error,
    ErrorCode,
    ErrorClass,
    Oid,
    OidType,
    HexOid,
    OidShortener<'static>,
    ObjectType,
    Libgit2,
    Features,
    Signature,
    SignatureRef<'static>,
    Time,
    // repository
    Repository,
    RepositoryInitOptions,
    RepositoryInitFlags,
    RepositoryInitMode,
    RepositoryOpenFlags,
    RepositoryState,
    RepositoryItem,
    RefStorage,
    // objects (immutable after parse, atomically counted)
    Object<'static>,
    Commit<'static>,
    Tree<'static>,
    TreeEntry,
    TreeEntryRef<'static>,
    TreeBuilder<'static>,
    TreeUpdate<'static>,
    TreeWalk,
    TreeWalkMode,
    Blob<'static>,
    BlobWriter<'static>,
    BlobFilterOptions,
    Tag<'static>,
    CommitBuilder<'static>,
    AmendOptions<'static>,
    ObjectIdOptions,
    // odb
    Odb<'static>,
    OdbObject,
    Mempack,
    LooseOptions,
    ExpandId,
    IndexerProgress,
    // config
    Config<'static>,
    ConfigEntries<'static>,
    ConfigEntry,
    ConfigLevel,
    ConfigLock<'static>,
    ConfigMap<'static>,
    ConfigMapKind,
    // index: every operation under the index's lock (I-1)
    Index<'static>,
    IndexEntries<'static>,
    IndexConflicts<'static>,
    IndexEntry,
    IndexTime,
    IndexCapability,
    IndexAddOption,
    IndexMatch,
    ConflictEntries,
    ReucEntry,
    NameEntry,
    Stage,
    // references
    Reference<'static>,
    ReferenceType,
    ReferenceFormat,
    Reflog<'static>,
    RefValue,
    ReflogRecord,
    RefdbUnlock,
    Sort,
    HashSig,
    HashSigOptions,
);

send_not_sync!(
    // driven by one thread at a time
    Backend,
    References<'static>,
    ReferenceNames<'static>,
    RefdbLock,
);

neither!(
    // streams over raw tables, single-threaded handles
    OdbWriter<'static>,
    OdbReader<'static>,
    OdbPackWriter<'static>,
    BackendRef<'static>,
    ReflogEntry<'static>,
    Refdb<'static>,
    Revwalk<'static>,
    // holds a boxed signing closure (not required to be Send)
    CommitCreateOptions<'static>,
);

#[test]
fn stated_at_compile_time() {
    // The assertions above are checked when this file compiles.
}
