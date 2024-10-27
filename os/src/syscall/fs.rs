//! File and filesystem-related syscalls

use crate::mm::translated_byte_buffer;
use crate::task::current_user_token;
use alloc::vec::Vec;

const FD_STDOUT: usize = 1;

/// write buf of length `len`  to a file with `fd`
pub fn sys_write(fd: usize, buf: *const u8, len: usize) -> isize {
    trace!("kernel: sys_write");
    match fd {
        FD_STDOUT => {
            let buffer = translated_byte_buffer(current_user_token(), buf, len).iter().map(|buffer| **buffer).collect::<Vec<u8>>();

            print!("{}", core::str::from_utf8(&buffer[..]).unwrap());
            len as isize
        }
        _ => {
            panic!("Unsupported fd in sys_write!");
        }
    }
}
