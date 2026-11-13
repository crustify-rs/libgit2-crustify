// A handle cannot outlive the box that frees the reference.
use {libgit2_crustify::libgit2::src::libgit2::refs::ReferenceBox, libgit2_crustify::libgit2::src::libgit2::refs::ReferenceRef};

fn handle<'a, 'r>(reference: ReferenceBox<'r>) -> ReferenceRef<'a, 'r> {
    reference.as_ref()
}

fn main() {}
