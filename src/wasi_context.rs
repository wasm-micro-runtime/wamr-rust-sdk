/*
 * Copyright (C) 2019 Intel Corporation. All rights reserved.
 * SPDX-License-Identifier: Apache-2.0 WITH LLVM-exception
 */

//! prepare wasi context

use alloc::{ffi::CString, vec::Vec};
use core::ffi::c_char;

#[derive(Debug, Default)]
struct PreOpen {
    real_paths: Vec<CString>,
    mapped_paths: Vec<CString>,
}

#[derive(Debug, Default)]
pub struct WasiCtxBuilder {
    pre_open: PreOpen,
    allowed_address: Vec<CString>,
    allowed_dns: Vec<CString>,
    env: Vec<CString>,
    // every element is a ptr to an env element
    env_cstr_ptrs: Vec<*const c_char>,
    args: Vec<CString>,
    // every element is a ptr to an args element
    args_cstr_ptrs: Vec<*const c_char>,
}

#[derive(Debug, Default)]
pub struct WasiCtx {
    pre_open: PreOpen,
    allowed_address: Vec<CString>,
    allowed_dns: Vec<CString>,
    env: Vec<CString>,
    env_cstr_ptrs: Vec<*const c_char>,
    args: Vec<CString>,
    args_cstr_ptrs: Vec<*const c_char>,
}

impl WasiCtxBuilder {
    pub fn new() -> WasiCtxBuilder {
        WasiCtxBuilder::default()
    }

    pub fn build(self) -> WasiCtx {
        WasiCtx {
            pre_open: self.pre_open,
            allowed_address: self.allowed_address,
            allowed_dns: self.allowed_dns,
            env: self.env,
            env_cstr_ptrs: self.env_cstr_ptrs,
            args: self.args,
            args_cstr_ptrs: self.args_cstr_ptrs,
        }
    }

    /// set pre-open directories and files, which are part of WASI arguments, for the module.
    /// the format of each map entry: <guest-path>::<host-path>
    ///
    /// This function should be called before `Instance::new`
    pub fn set_pre_open_path(
        mut self,
        real_paths: Vec<&str>,
        mapped_paths: Vec<&str>,
    ) -> WasiCtxBuilder {
        self.pre_open.real_paths = real_paths
            .iter()
            .map(|s| CString::new(s.as_bytes()).unwrap())
            .collect::<Vec<CString>>();

        self.pre_open.mapped_paths = mapped_paths
            .iter()
            .map(|s| CString::new(s.as_bytes()).unwrap())
            .collect::<Vec<CString>>();

        self
    }

    /// set environment variables, which are part of WASI arguments, for the module
    ///
    /// This function should be called before `Instance::new`
    ///
    /// all wasi args of a module will be spread into the environment variables of the module
    pub fn set_env_vars(mut self, envs: Vec<&str>) -> WasiCtxBuilder {
        self.env = envs
            .iter()
            .map(|s| CString::new(s.as_bytes()).unwrap())
            .collect::<Vec<CString>>();
        self.env_cstr_ptrs = self
            .env
            .iter()
            .map(|s| s.as_ptr() as *const c_char)
            .collect::<Vec<*const c_char>>();

        self
    }

    /// set allowed ns , which are part of WASI arguments, for the module
    ///
    /// This function should be called before `Instance::new`
    pub fn set_allowed_dns(mut self, dns: Vec<&str>) -> WasiCtxBuilder {
        self.allowed_dns = dns
            .iter()
            .map(|s| CString::new(s.as_bytes()).unwrap())
            .collect::<Vec<CString>>();

        self
    }

    /// set allowed ip addresses, which are part of WASI arguments, for the module
    ///
    /// This function should be called before `Instance::new`
    pub fn set_allowed_address(mut self, addresses: Vec<&str>) -> WasiCtxBuilder {
        self.allowed_address = addresses
            .iter()
            .map(|s| CString::new(s.as_bytes()).unwrap())
            .collect::<Vec<CString>>();

        self
    }

    /// set arguments, which are part of WASI arguments, for the module
    ///
    /// This function should be called before `Instance::new`
    pub fn set_arguments(mut self, args: Vec<&str>) -> WasiCtxBuilder {
        self.args = args
            .iter()
            .map(|s| CString::new(s.as_bytes()).unwrap())
            .collect::<Vec<CString>>();
        self.args_cstr_ptrs = self
            .args
            .iter()
            .map(|s| s.as_ptr() as *const c_char)
            .collect::<Vec<*const c_char>>();

        self
    }
}

impl WasiCtx {
    pub fn get_preopen_real_paths(&self) -> &Vec<CString> {
        &self.pre_open.real_paths
    }

    pub fn get_preopen_mapped_paths(&self) -> &Vec<CString> {
        &self.pre_open.mapped_paths
    }

    pub fn get_allowed_address(&self) -> &Vec<CString> {
        &self.allowed_address
    }

    pub fn get_allowed_dns(&self) -> &Vec<CString> {
        &self.allowed_dns
    }

    pub fn get_env_vars(&self) -> &Vec<CString> {
        &self.env
    }

    pub fn get_env_vars_ptrs(&self) -> &Vec<*const c_char> {
        &self.env_cstr_ptrs
    }

    pub fn get_arguments(&self) -> &Vec<CString> {
        &self.args
    }

    pub fn get_arguments_ptrs(&self) -> &Vec<*const c_char> {
        &self.args_cstr_ptrs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_wasi_ctx_build() {
        let wasi_ctx = WasiCtxBuilder::new()
            .set_pre_open_path(vec!["a/b/c"], vec!["/dog/cat/rabbit"])
            .set_allowed_address(vec!["1.2.3.4"])
            .set_allowed_dns(vec![])
            .set_env_vars(vec!["path=/usr/local/bin", "HOME=/home/xxx"])
            .set_arguments(vec!["arg1", "arg2"])
            .build();

        let mut preopen_iter = wasi_ctx.get_preopen_real_paths().iter();
        assert_eq!(preopen_iter.next().unwrap().to_str().unwrap(), "a/b/c");
        assert_eq!(preopen_iter.next(), None);

        let mut preopen_iter = wasi_ctx.get_preopen_mapped_paths().iter();
        assert_eq!(
            preopen_iter.next().unwrap().to_str().unwrap(),
            "/dog/cat/rabbit"
        );
        assert_eq!(preopen_iter.next(), None);

        let mut allowed_address_iter = wasi_ctx.get_allowed_address().iter();
        assert_eq!(
            allowed_address_iter.next().unwrap().to_str().unwrap(),
            "1.2.3.4"
        );
        assert_eq!(allowed_address_iter.next(), None);

        let mut env_vars_iter = wasi_ctx.get_env_vars().iter();
        assert_eq!(
            env_vars_iter.next().unwrap().to_str().unwrap(),
            "path=/usr/local/bin"
        );
        assert_eq!(
            env_vars_iter.next().unwrap().to_str().unwrap(),
            "HOME=/home/xxx"
        );
        assert_eq!(env_vars_iter.next(), None);
    }
}
