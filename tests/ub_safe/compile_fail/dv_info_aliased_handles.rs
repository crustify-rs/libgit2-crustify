// A shared handle cannot coexist with the exclusive one of the same record.
use libgit2_crustify::libgit2::src::util::hash::sha1dc::ubc_check::DvInfo;

fn main() {
    let mut record = DvInfo::new(1, 43, 0, 58, 0, 0, [0; 80]).unwrap();
    let shared = record.as_ref();
    let exclusive = record.as_mut();
    let _ = shared.dv_type();
    drop(exclusive);
}
