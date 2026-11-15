// the cost of a thread-local lookup as one thread
// repeats it, as C's threadstate_get does through git_tlsdata_get on every git_error_clear.
use libgit2_crustify::libgit2::src::util::thread::TlsKey;
use std::time::Instant;

fn main() {
    let key: TlsKey<usize> = TlsKey::new(None).unwrap();
    key.set(Some(7)).unwrap();
    let chunk: usize = std::env::args().nth(1).map_or(2000, |a| a.parse().unwrap());
    let chunks: usize = std::env::args().nth(2).map_or(8, |a| a.parse().unwrap());
    let mut s = 0;
    for c in 0..chunks {
        let t = Instant::now();
        for _ in 0..chunk {
            s += key.with(|v| *v.unwrap());
        }
        println!("chunk {c}: {:.1} us per lookup", t.elapsed().as_secs_f64() * 1e6 / chunk as f64);
    }
    assert_eq!(s, 7 * chunk * chunks);
}
