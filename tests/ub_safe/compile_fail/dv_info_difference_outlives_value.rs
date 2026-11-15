// The message difference a record lends cannot outlive the record.
use libgit2_crustify::libgit2::src::util::hash::sha1dc::ubc_check::DvInfo;

fn main() {
    let dm = {
        let record = DvInfo::new(1, 43, 0, 58, 0, 0, [0; 80]).unwrap();
        record.as_ref().message_difference()
    };
    let _ = dm[0];
}
