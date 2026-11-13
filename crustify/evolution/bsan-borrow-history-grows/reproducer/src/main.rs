//! Times the loop of `strnlen_h`'s exhaustive tests per word offset: `-- <starts>` reuses one
//! buffer for every offset, `-- <starts> fresh` gives each offset its own. Under `cargo bsan run`
//! (debug) the first offset costs as much either way, and each later one grows only when reused.
use core::ffi::CStr;
use std::time::Instant;

const fn strnlen(bytes: &[u8]) -> usize {
    match CStr::from_bytes_until_nul(bytes) {
        Ok(string) => string.count_bytes(),
        Err(_) => bytes.len(),
    }
}

fn main() {
    let starts: usize = std::env::args().nth(1).map_or(8, |s| s.parse().unwrap());
    let fresh = std::env::args().nth(2).is_some();
    let mut buf = [b'x'; 80];
    let all = Instant::now();
    let mut runs = 0u64;
    for start in 0..starts {
        let t = Instant::now();
        let mut local = [b'x'; 80];
        let buf: &mut [u8; 80] = if fresh { &mut local } else { &mut buf };
        for len in 0..=70 {
            for nul in (0..len).map(Some).chain([None]) {
                buf.fill(b'x');
                if let Some(at) = nul {
                    buf[start + at] = 0;
                }
                let run = &buf[start..start + len];
                assert_eq!(strnlen(run), nul.unwrap_or(len));
                assert_eq!(strnlen(run), run.iter().position(|&b| b == 0).unwrap_or(run.len()));
                runs += 1;
            }
        }
        eprintln!("start {start}: {:?}", t.elapsed());
    }
    eprintln!("{runs} runs in {:?}", all.elapsed());
}
