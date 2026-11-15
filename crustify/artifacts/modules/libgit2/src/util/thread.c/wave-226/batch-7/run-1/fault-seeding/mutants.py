#!/usr/bin/env python3
"""Writes mutants.json, the faults seeded in thread.rs, and checks each site is unique."""
import json
import sys

M = []


def m(id, kind, desc, old, new):
    M.append({"id": id, "kind": kind, "desc": desc, "old": old, "new": new})


# --- The native atomics (Atomic32Ref, Atomic64Ref, AtomicWord) ---
m("a32-inc-old", "return", "inc returns the value it found",
  "self.val().fetch_add(1, SeqCst).wrapping_add(1)", "self.val().fetch_add(1, SeqCst)")
m("a32-inc-by2", "constant", "inc adds 2",
  "self.val().fetch_add(1, SeqCst).wrapping_add(1)", "self.val().fetch_add(2, SeqCst).wrapping_add(1)")
m("a32-inc-sat", "operator", "inc's result saturates",
  "self.val().fetch_add(1, SeqCst).wrapping_add(1)", "self.val().fetch_add(1, SeqCst).saturating_add(1)")
m("a32-dec-old", "return", "dec returns the value it found",
  "self.val().fetch_sub(1, SeqCst).wrapping_sub(1)", "self.val().fetch_sub(1, SeqCst)")
m("a32-dec-add", "operator", "dec adds",
  "self.val().fetch_sub(1, SeqCst).wrapping_sub(1)", "self.val().fetch_add(1, SeqCst).wrapping_sub(1)")
m("a32-dec-sat", "operator", "dec's result saturates",
  "self.val().fetch_sub(1, SeqCst).wrapping_sub(1)", "self.val().fetch_sub(1, SeqCst).saturating_sub(1)")
m("a32-add-old", "return", "add32 returns the value it found",
  "pub fn add(&self, addend: i32) -> i32 {\n        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)",
  "pub fn add(&self, addend: i32) -> i32 {\n        self.val().fetch_add(addend, SeqCst)")
m("a32-add-sub", "operator", "add32 subtracts",
  "pub fn add(&self, addend: i32) -> i32 {\n        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)",
  "pub fn add(&self, addend: i32) -> i32 {\n        self.val().fetch_sub(addend, SeqCst).wrapping_sub(addend)")
m("a32-add-sat", "operator", "add32's result saturates",
  "pub fn add(&self, addend: i32) -> i32 {\n        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)",
  "pub fn add(&self, addend: i32) -> i32 {\n        self.val().fetch_add(addend, SeqCst).saturating_add(addend)")
m("a32-get-bit", "return", "get32 flips the low bit",
  "pub fn get(&self) -> i32 {\n        self.val().load(SeqCst)",
  "pub fn get(&self) -> i32 {\n        self.val().load(SeqCst) ^ 1")
m("a32-set-plus1", "constant", "set32 stores val + 1",
  "pub fn set(&self, val: i32) {\n        self.val().store(val, SeqCst);",
  "pub fn set(&self, val: i32) {\n        self.val().store(val.wrapping_add(1), SeqCst);")
m("a64-add-old", "return", "add64 returns the value it found",
  "pub fn add(&self, addend: i64) -> i64 {\n        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)",
  "pub fn add(&self, addend: i64) -> i64 {\n        self.val().fetch_add(addend, SeqCst)")
m("a64-add-sat", "operator", "add64's result saturates",
  "pub fn add(&self, addend: i64) -> i64 {\n        self.val().fetch_add(addend, SeqCst).wrapping_add(addend)",
  "pub fn add(&self, addend: i64) -> i64 {\n        self.val().fetch_add(addend, SeqCst).saturating_add(addend)")
m("a64-get-bit", "return", "get64 flips the low bit",
  "pub fn get(&self) -> i64 {\n        self.val().load(SeqCst)",
  "pub fn get(&self) -> i64 {\n        self.val().load(SeqCst) ^ 1")
m("a64-set-plus1", "constant", "set64 stores val + 1",
  "pub fn set(&self, val: i64) {\n        self.val().store(val, SeqCst);",
  "pub fn set(&self, val: i64) {\n        self.val().store(val.wrapping_add(1), SeqCst);")
m("cas-new", "return", "a successful compare-and-swap returns the new value",
  "Ok(found) | Err(found) => found,", "Ok(_) => new,\n                    Err(found) => found,")
m("cas-old", "return", "a compare-and-swap returns the expected value either way",
  "Ok(found) | Err(found) => found,", "Ok(_) | Err(_) => old,")
m("cas-unconditional", "comparison", "the compare-and-swap stores whatever the slot held",
  "match self.compare_exchange(old, new, SeqCst, SeqCst) {",
  "match Ok::<$value, $value>(self.swap(new, SeqCst)) {")
m("swap-new", "return", "swap returns the new value",
  "<$atomic>::swap(self, new, SeqCst)", "{\n                <$atomic>::swap(self, new, SeqCst);\n                new\n            }")
m("swap-nostore", "statement", "swap does not store",
  "<$atomic>::swap(self, new, SeqCst)", "{\n                let _ = new;\n                <$atomic>::load(self, SeqCst)\n            }")
m("load-cas", "statement", "load stores the slot's value back by compare-and-swap of a stale guess",
  "fn load(&self) -> $value {\n                <$atomic>::load(self, SeqCst)",
  "fn load(&self) -> $value {\n                let v = <$atomic>::load(self, SeqCst);\n                <$atomic>::swap(self, v, SeqCst)")

# --- The gateway's exports ---
m("x-get32", "return", "git_atomic32_get flips the low bit",
  "unsafe { atomic32(a) }.get()", "unsafe { atomic32(a) }.get() ^ 1")
m("x-set32", "constant", "git_atomic32_set stores val ^ 1",
  "unsafe { atomic32(a) }.set(val);", "unsafe { atomic32(a) }.set(val ^ 1);")
m("x-inc32", "operator", "git_atomic32_inc adds 2",
  "unsafe { atomic32(a) }.inc()", "unsafe { atomic32(a) }.add(2)")
m("x-dec32", "operator", "git_atomic32_dec increments",
  "unsafe { atomic32(a) }.dec()", "unsafe { atomic32(a) }.inc()")
m("x-add32", "operator", "git_atomic32_add subtracts",
  "unsafe { atomic32(a) }.add(addend)", "unsafe { atomic32(a) }.add(addend.wrapping_neg())")
m("x-get64", "return", "git_atomic64_get flips the low bit",
  "unsafe { atomic64(a) }.get()", "unsafe { atomic64(a) }.get() ^ 1")
m("x-set64", "constant", "git_atomic64_set stores val ^ 1",
  "unsafe { atomic64(a) }.set(val);", "unsafe { atomic64(a) }.set(val ^ 1);")
m("x-add64", "operator", "git_atomic64_add subtracts",
  "unsafe { atomic64(a) }.add(addend)", "unsafe { atomic64(a) }.add(addend.wrapping_neg())")
m("x-cas", "comparison", "git_atomic__compare_and_swap swaps unconditionally",
  "atomic_compare_and_swap(unsafe { slot(ptr) }, oldval, newval)",
  "{\n            let _ = oldval;\n            atomic_swap(unsafe { slot(ptr) }, newval)\n        }")
m("x-swap", "statement", "git_atomic__swap only loads",
  "atomic_swap(unsafe { slot(ptr) }, newval)", "{\n            let _ = newval;\n            atomic_load(unsafe { slot(ptr) })\n        }")
m("x-load", "statement", "git_atomic__load empties the slot",
  "atomic_load(unsafe { slot(ptr) })", "atomic_swap(unsafe { slot(ptr) }, ptr::null_mut())")
m("x-global-init", "constant", "git_threads_global_init returns 1",
  "global_init();\n        0", "global_init();\n        1")
m("x-init-id", "constant", "git_tlsdata_init writes the next id",
  "unsafe { key.write(id) };", "unsafe { key.write(id + 1) };")
m("x-init-nodtor", "statement", "git_tlsdata_init drops the destructor",
  "let destructor = destroy_fn.map(|f| unsafe { CDestructor::new(f) });",
  "let destructor: Option<CDestructor> = {\n            let _ = destroy_fn;\n            None\n        };")
m("x-set-ok", "return", "git_tlsdata_set reports success on failure",
  "if barrier(|| c_keys::set(key, word).is_ok(), || false) {",
  "if barrier(|| c_keys::set(key, word).is_ok() || true, || false) {")
m("x-set-null", "statement", "git_tlsdata_set stores NULL",
  "let word = NonNull::new(value).map(|value| unsafe { CWord::new(value) });",
  "let word: Option<CWord> = {\n            let _ = value;\n            None\n        };")
m("x-get-null", "return", "git_tlsdata_get returns NULL",
  "|| c_keys::get(key).map_or(ptr::null_mut(), CWord::as_ptr),",
  "|| {\n                let _ = c_keys::get(key);\n                ptr::null_mut()\n            },")
m("x-dispose-ok", "return", "git_tlsdata_dispose reports success on failure",
  "if barrier(|| c_keys::dispose(key).is_ok(), || false) {",
  "if barrier(|| c_keys::dispose(key).is_ok() || true, || false) {")

# --- Thread-local storage ---
m("t-drop-no-c", "statement", "a thread's exit runs no C destructor",
  "(Value::C(word), Destructor::C(Some(destructor))) => destructor.destroy(word),",
  "(Value::C(_), Destructor::C(Some(_))) => {}")
m("t-drop-no-rust", "statement", "a thread's exit runs no Rust destructor",
  "(Value::Rust(value), Destructor::Rust { run: Some(run), .. }) => run(value),",
  "(Value::Rust(value), Destructor::Rust { run: Some(run), .. }) => drop((value, run)),")
m("t-drop-dead", "comparison", "a thread's exit destroys the values of disposed keys",
  "                if !key.live.load(Ordering::Acquire) {\n                    drop(value);\n                    continue;\n                }\n",
  "")
m("t-replace-none", "return", "replacing a value returns nothing",
  "Some(value) => Ok(Some(mem::replace(&mut table.entries[index].1, value))),",
  "Some(value) => {\n                        table.entries[index].1 = value;\n                        Ok(None)\n                    }")
m("t-remove-keeps", "statement", "storing NULL keeps the value",
  "None => Ok(Some(table.entries.swap_remove(index).1)),", "None => Ok(None),")
m("t-set-kind", "comparison", "a store ignores the key's kind",
  "(Some(index), value) if table.entries[index].0.kind == kind =>", "(Some(index), value) if true =>")
m("t-get-holds", "comparison", "a read ignores the key's state",
  "if key.holds(kind) { read(value) } else { None }", "read(value)")
m("t-first-id-0", "constant", "the first id is 0",
  "next: 1,\n                live: BTreeMap::new(),", "next: 0,\n                live: BTreeMap::new(),")
m("t-reuse-id", "statement", "a disposed key's id is handed out again",
  "if let Some(key) = registry.live.remove(&id) {",
  "if let Some(key) = registry.live.remove(&id) {\n            registry.next = id;")
m("t-dispose-live", "constant", "disposing leaves the key live",
  "key.live.store(false, Ordering::Release);", "key.live.store(true, Ordering::Release);")
m("t-holds-kind", "comparison", "a key holds any kind",
  "self.kind == kind && self.live.load(Ordering::Acquire)", "self.live.load(Ordering::Acquire)")
m("t-holds-live", "comparison", "a disposed key still holds",
  "self.kind == kind && self.live.load(Ordering::Acquire)", "self.kind == kind")
m("t-prune-keeps", "comparison", "pruning keeps every entry",
  ".partition(|(key, _)| key.live.load(Ordering::Acquire));", ".partition(|_| true);")
m("t-prune-any", "comparison", "pruning stops when any entry is live",
  ".all(|(key, _)| key.live.load(Ordering::Acquire))", ".any(|(key, _)| key.live.load(Ordering::Acquire))")
m("t-first-store-any", "comparison", "a first store takes any registered id, of any kind",
  "let Some(key) = registry().key(id, kind).cloned() else {",
  "let Some(key) = registry().live.get(&id).cloned() else {")
m("t-dispose-twice", "comparison", "disposing of a key twice succeeds",
  "        if !registry.holds(id, kind) {\n            return Err(TlsError::Disposed);\n        }\n", "")
m("k-set-prev", "return", "TlsKey::set returns no previous value",
  "previous.downcast::<T>().ok().map(|previous| *previous)", "{\n                let _ = previous;\n                None\n            }")
m("k-dtor-skip", "statement", "TlsKey's destructor is never called",
  "if let Ok(value) = value.downcast::<T>() {\n                    f(*value);\n                }",
  "let _ = (value, f);")
m("k-with-none", "return", "TlsKey::with lends None",
  ".and_then(|value| f.take().map(|f| f(Some(value)))),", ".and_then(|_| f.take().map(|f| f(None))),")

# --- Locks and threads ---
m("l-notify", "statement", "notify_one wakes nobody",
  "    pub fn notify_one(&self) {\n        self.inner.notify_one();", "    pub fn notify_one(&self) {\n        let _ = &self.inner;")
m("l-mutex-unlock", "statement", "MutexGuard::unlock leaks the guard",
  "    pub fn unlock(guard: Self) {\n        drop(guard);\n    }\n}\n\nimpl<T: ?Sized> Deref for MutexGuard",
  "    pub fn unlock(guard: Self) {\n        core::mem::forget(guard);\n    }\n}\n\nimpl<T: ?Sized> Deref for MutexGuard")
m("l-poisoned", "return", "Mutex::is_poisoned is false",
  "    pub fn is_poisoned(&self) -> bool {\n        self.inner.is_poisoned()\n    }\n\n    /// The data, through the exclusive borrow of the lock, which needs no locking.\n    ///\n    /// # Errors\n    ///\n    /// When the lock is poisoned, the [`PoisonError`] holds the data.\n    pub fn get_mut(&mut self) -> LockResult<&mut T> {\n        self.inner.get_mut()\n    }\n}\n\nimpl<T: Default> Default for Mutex<T>",
  "    pub fn is_poisoned(&self) -> bool {\n        let _ = &self.inner;\n        false\n    }\n\n    /// The data, through the exclusive borrow of the lock, which needs no locking.\n    ///\n    /// # Errors\n    ///\n    /// When the lock is poisoned, the [`PoisonError`] holds the data.\n    pub fn get_mut(&mut self) -> LockResult<&mut T> {\n        self.inner.get_mut()\n    }\n}\n\nimpl<T: Default> Default for Mutex<T>")
m("h-set-thread", "constant", "set_thread stores the id ^ 1",
  "unsafe { slot.write(thread) }", "unsafe { slot.write(thread ^ 1) }")
m("h-thread", "return", "ThreadRef::thread reads the id ^ 1",
  "unsafe { thread.read() }", "unsafe { thread.read() ^ 1 }")
m("h-finished", "return", "is_finished is inverted",
  "self.inner.is_finished()", "!self.inner.is_finished()")

text = open(sys.argv[1]).read()
bad = [(x["id"], text.count(x["old"])) for x in M if text.count(x["old"]) != 1]
if bad:
    sys.exit(f"sites not unique: {bad}")
json.dump(M, open(sys.argv[2], "w"), indent=1)
print(len(M), "mutants")
