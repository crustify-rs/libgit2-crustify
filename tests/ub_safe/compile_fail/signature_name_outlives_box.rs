// A borrowed name cannot outlive the signature that owns it.
use libgit2_crustify::libgit2::src::libgit2::signature::Signature;

fn main() {
    let name = {
        let sig = Signature::new(c"A", c"a@b", 0, 0).unwrap();
        sig.as_ref().name()
    };
    let _ = name;
}
