//! What the object modules (object, object_api, blob, tag, signature,
//! grafts) use of the modules around them: their types under the C names
//! the object code was written against, and their Rust definitions of the
//! C ABI (the object code was written over C-shaped calls).

#![allow(non_camel_case_types)]

pub use crate::cache::GitCache as git_cache;
pub use crate::config::GitConfig as git_config;
pub use crate::odb::{
    GitOdb as git_odb, GitOdbObject as git_odb_object, GitOdbStream as git_odb_stream,
};
pub use crate::refs::GitReference as git_reference;
pub use crate::repository::GitRepository as git_repository;
pub use crate::tree::GitTreeEntry as git_tree_entry;

pub use crate::commit::ffi::{
    git_commit__free, git_commit__parse, git_commit__parse_raw, git_commit_tree,
};
pub use crate::config::ffi::{git_config_free, git_config_get_string};
pub use crate::odb::ffi::{
    git_odb_free, git_odb_object_data, git_odb_object_free, git_odb_open_wstream, git_odb_read,
    git_odb_stream_finalize_write, git_odb_stream_free, git_odb_stream_write,
};
pub use crate::refs::ffi::{
    git_reference_create, git_reference_delete, git_reference_foreach_name, git_reference_free,
    git_reference_lookup, git_reference_name_is_valid, git_reference_name_to_id,
};
pub use crate::repository::ffi::{
    git_repository__abbrev_length, git_repository__item_path, git_repository_config_snapshot,
    git_repository_odb, git_repository_workdir_path,
};
pub use crate::tree::ffi::{
    git_tree__free, git_tree__parse, git_tree__parse_raw, git_tree_entry_bypath,
    git_tree_entry_free, git_tree_entry_to_object, git_tree_entry_type,
};
