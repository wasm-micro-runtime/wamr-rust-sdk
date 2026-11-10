/*
 * Copyright (C) 2019 Intel Corporation. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

use alloc::{string::{String, ToString}, vec::Vec};
use core::ffi::{c_char, CStr};

pub const DEFAULT_ERROR_BUF_SIZE: usize = 128;

pub fn error_buf_to_string(&error_buf: &[c_char; DEFAULT_ERROR_BUF_SIZE]) -> String {
    let error_content: Vec<u8> = error_buf
        .map(|c| c as u8)
        .into_iter()
        .filter(|c| *c > 0)
        .collect();
    String::from_utf8(error_content).unwrap()
}

pub fn cstr_to_string(raw_cstr: *const c_char) -> String {
    let cstr = unsafe { CStr::from_ptr(raw_cstr) };
    String::from_utf8_lossy(cstr.to_bytes()).to_string()
}

pub fn exception_to_string(raw_exception: *const c_char) -> String {
    cstr_to_string(raw_exception)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::CString;

    #[test]
    fn test_error_buf_empty() {
        let error_buf = [0 as c_char; DEFAULT_ERROR_BUF_SIZE];
        let error_str = error_buf_to_string(&error_buf);
        assert_eq!(error_str.len(), 0);
        assert_eq!(error_str, "");
    }

    #[test]
    fn test_error_buf() {
        let mut error_buf = [0 as c_char; DEFAULT_ERROR_BUF_SIZE];
        error_buf[0] = 'a' as i8;
        error_buf[1] = 'b' as i8;
        error_buf[2] = 'c' as i8;

        let error_str = error_buf_to_string(&error_buf);
        assert_eq!(error_str.len(), 3);
        assert_eq!(error_str, "abc");
    }

    #[test]
    fn test_exception_to_string() {
        let exception = "it is an exception";

        let exception_cstr = CString::new(exception).expect("CString::new failed");
        let exception_str = exception_to_string(exception_cstr.as_ptr());
        assert_eq!(exception_str.len(), exception.len());
        assert_eq!(exception_str, exception);
    }
}
