//! Thin safe wrapper over `libsqlite3-sys` 0.37.
//!
//! We can't use `rusqlite` directly: it pins `libsqlite3-sys ^0.36` while
//! `whatsapp-rust-sqlite-storage` pulls `^0.37`, and both crates `links =
//! "sqlite3"`, so cargo refuses two copies of the native lib. This wrapper
//! exposes just enough of the C API for the session/webhook/contacts
//! queries and rides on the same `libsqlite3-sys` 0.37 the upstream
//! whatsapp-rust storage already brings in.

use libsqlite3_sys::*;
use parking_lot::Mutex;
use std::ffi::{CStr, CString};
use std::os::raw::{c_int, c_void};
use std::ptr;
use std::sync::Arc;

/// An owned SQLite connection. Safe to share across threads via `Arc`;
/// SQLite is configured with FULLMUTEX so internal locking is per-call,
/// but we still take an exclusive `parking_lot::Mutex` around every
/// statement so prepared-statement state can't tangle across tasks.
pub struct Connection {
    handle: *mut sqlite3,
}

unsafe impl Send for Connection {}
unsafe impl Sync for Connection {}

impl Drop for Connection {
    fn drop(&mut self) {
        unsafe {
            sqlite3_close(self.handle);
        }
    }
}

/// Thread-safe handle suitable for stashing in `DbPool::SQLite`.
pub type SqliteHandle = Arc<Mutex<Connection>>;

pub fn open(path: &str) -> anyhow::Result<SqliteHandle> {
    let path_c = CString::new(path)?;
    let mut handle: *mut sqlite3 = ptr::null_mut();
    let rc = unsafe {
        sqlite3_open_v2(
            path_c.as_ptr(),
            &mut handle,
            SQLITE_OPEN_READWRITE | SQLITE_OPEN_CREATE | SQLITE_OPEN_FULLMUTEX,
            ptr::null(),
        )
    };
    if rc != SQLITE_OK {
        return Err(anyhow::anyhow!(
            "sqlite3_open_v2 failed for {}: rc={}",
            path,
            rc
        ));
    }
    let conn = Connection { handle };
    let arc = Arc::new(Mutex::new(conn));
    Ok(arc)
}

/// Run a list of SQL statements separated by `;` for schema init.
pub fn exec_batch(conn: &Connection, sql: &str) -> anyhow::Result<()> {
    let sql_c = CString::new(sql)?;
    let mut err: *mut std::os::raw::c_char = ptr::null_mut();
    let rc = unsafe { sqlite3_exec(conn.handle, sql_c.as_ptr(), None, ptr::null_mut(), &mut err) };
    if rc != SQLITE_OK {
        let msg = unsafe {
            if err.is_null() {
                "unknown".to_string()
            } else {
                let s = CStr::from_ptr(err).to_string_lossy().to_string();
                sqlite3_free(err as *mut c_void);
                s
            }
        };
        return Err(anyhow::anyhow!("sqlite exec failed: {}", msg));
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum Value {
    Null,
    Int(i64),
    Text(String),
}

impl Value {
    pub fn from_opt_str(v: Option<&str>) -> Value {
        match v {
            Some(s) => Value::Text(s.to_string()),
            None => Value::Null,
        }
    }
}

fn bind(stmt: *mut sqlite3_stmt, idx: c_int, v: &Value) -> anyhow::Result<()> {
    let rc = unsafe {
        match v {
            Value::Null => sqlite3_bind_null(stmt, idx),
            Value::Int(i) => sqlite3_bind_int64(stmt, idx, *i),
            Value::Text(s) => {
                let n = s.len() as c_int;
                let ptr = s.as_ptr() as *const std::os::raw::c_char;
                // SQLITE_TRANSIENT = -1 cast, makes SQLite copy the bytes.
                sqlite3_bind_text(stmt, idx, ptr, n, SQLITE_TRANSIENT())
            }
        }
    };
    if rc != SQLITE_OK {
        return Err(anyhow::anyhow!("sqlite bind failed: rc={}", rc));
    }
    Ok(())
}

/// Run a parameterised statement that doesn't return rows. Returns the
/// number of changed rows reported by `sqlite3_changes`.
pub fn execute(conn: &Connection, sql: &str, params: &[Value]) -> anyhow::Result<u64> {
    let sql_c = CString::new(sql)?;
    let mut stmt: *mut sqlite3_stmt = ptr::null_mut();
    let rc =
        unsafe { sqlite3_prepare_v2(conn.handle, sql_c.as_ptr(), -1, &mut stmt, ptr::null_mut()) };
    if rc != SQLITE_OK {
        let msg = errmsg(conn.handle);
        return Err(anyhow::anyhow!("sqlite prepare failed: {} :: {}", msg, sql));
    }
    let res = (|| -> anyhow::Result<u64> {
        for (i, v) in params.iter().enumerate() {
            bind(stmt, (i + 1) as c_int, v)?;
        }
        let rc = unsafe { sqlite3_step(stmt) };
        if rc != SQLITE_DONE && rc != SQLITE_ROW {
            return Err(anyhow::anyhow!(
                "sqlite step failed: {}",
                errmsg(conn.handle)
            ));
        }
        let changes = unsafe { sqlite3_changes(conn.handle) } as u64;
        Ok(changes)
    })();
    unsafe {
        sqlite3_finalize(stmt);
    }
    res
}

/// Run a parameterised query and call `map` for each row. Returns the
/// collected results.
pub fn query<F, T>(
    conn: &Connection,
    sql: &str,
    params: &[Value],
    mut map: F,
) -> anyhow::Result<Vec<T>>
where
    F: FnMut(&Row) -> T,
{
    let sql_c = CString::new(sql)?;
    let mut stmt: *mut sqlite3_stmt = ptr::null_mut();
    let rc =
        unsafe { sqlite3_prepare_v2(conn.handle, sql_c.as_ptr(), -1, &mut stmt, ptr::null_mut()) };
    if rc != SQLITE_OK {
        let msg = errmsg(conn.handle);
        return Err(anyhow::anyhow!("sqlite prepare failed: {} :: {}", msg, sql));
    }
    let res = (|| -> anyhow::Result<Vec<T>> {
        for (i, v) in params.iter().enumerate() {
            bind(stmt, (i + 1) as c_int, v)?;
        }
        let mut out = Vec::new();
        loop {
            let rc = unsafe { sqlite3_step(stmt) };
            if rc == SQLITE_ROW {
                let row = Row { stmt };
                out.push(map(&row));
            } else if rc == SQLITE_DONE {
                break;
            } else {
                return Err(anyhow::anyhow!(
                    "sqlite step failed: {}",
                    errmsg(conn.handle)
                ));
            }
        }
        Ok(out)
    })();
    unsafe {
        sqlite3_finalize(stmt);
    }
    res
}

pub struct Row {
    stmt: *mut sqlite3_stmt,
}

impl Row {
    pub fn get_string(&self, i: c_int) -> Option<String> {
        unsafe {
            let t = sqlite3_column_type(self.stmt, i);
            if t == SQLITE_NULL {
                return None;
            }
            let ptr = sqlite3_column_text(self.stmt, i);
            if ptr.is_null() {
                return None;
            }
            let bytes = sqlite3_column_bytes(self.stmt, i) as usize;
            let slice = std::slice::from_raw_parts(ptr, bytes);
            Some(String::from_utf8_lossy(slice).to_string())
        }
    }
    pub fn get_int(&self, i: c_int) -> i64 {
        unsafe { sqlite3_column_int64(self.stmt, i) }
    }
}

fn errmsg(handle: *mut sqlite3) -> String {
    unsafe {
        let p = sqlite3_errmsg(handle);
        if p.is_null() {
            return "?".to_string();
        }
        CStr::from_ptr(p).to_string_lossy().to_string()
    }
}

// SQLITE_TRANSIENT lives as a constant function pointer in C; expose it.
#[allow(non_snake_case)]
fn SQLITE_TRANSIENT() -> sqlite3_destructor_type {
    Some(unsafe { std::mem::transmute::<isize, unsafe extern "C" fn(*mut c_void)>(-1) })
}
