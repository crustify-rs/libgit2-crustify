// Id options cannot outlive the filter list they borrow.
use libgit2_crustify::sys::libgit2 as sys;
use libgit2_crustify::libgit2::src::libgit2::filter::FilterList;
use libgit2_crustify::libgit2::src::libgit2::object::ObjectIdOptions;
use libgit2_crustify::libgit2::src::libgit2::repository::Repository;

fn main() {
    let repo = Repository::new().unwrap();
    let mut opts = ObjectIdOptions::new();
    {
        let mut list = FilterList::new(repo.as_ref(), sys::GIT_FILTER_TO_ODB, 0).unwrap();
        opts.as_mut().set_filters(Some(list.as_mut()));
    }
    println!("{:?}", opts.as_ref().filters());
}
