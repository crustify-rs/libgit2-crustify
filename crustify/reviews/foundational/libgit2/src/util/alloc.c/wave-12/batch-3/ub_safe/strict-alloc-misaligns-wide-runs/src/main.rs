//! `AllocVec<u128>` from the safe API alone, on a libgit2 configured with
//! `-DDEBUG_STRICT_ALLOC=ON`: its default allocator (`debugalloc`) returns `malloc`'s block
//! offset by a `size_t`, 8-aligned, while `ArrayFree` vouches for 16 (`MAX_ALIGN`), so the
//! run's slice is misaligned for `u128`.
#![forbid(unsafe_code)]

use libgit2::libgit2::src::util::alloc::{AllocVec, ArrayFree};

fn main() {
    let run = AllocVec::<u128>::from_slice(&[1, 2]).expect("allocation");
    println!("from_slice: {:p}", run.as_ptr());
    let zeroed = ArrayFree::new_zeroed::<u128>(2).expect("allocation");
    println!("new_zeroed: {:p}", zeroed.as_ptr());
    // Forms `&[u128]` over the misaligned block.
    let sum: u128 = run.as_slice().iter().chain(zeroed.as_slice()).sum();
    assert_eq!(sum, 3);
}
