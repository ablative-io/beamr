%% WPORT-7 PROBE-FAILURE section 1 workload (harness-authored, uncommitted).
%%
%% Depends on the SCRATCH native BIF `probe:panic_now/0` (see panic-source.diff)
%% which panics with `panic!("wport7 intentional panic wall probe")` — the same
%% message style as the cfg(test) panic wall (`failure_tests.rs`
%% `panicking_test_bif`). Call `WasmVm.install_probe_panic_bif()` BEFORE loading
%% this module so the `probe:panic_now/0` import resolves at load time.
-module(panic_probe).
-export([boom/0, wait_boom/0]).

%% SYNC entry (WPORT-7 1a): a direct import call that panics immediately when
%% the process is driven by `run_step()`. The trap surfaces synchronously to
%% the caller (caught in a try/catch, or uncaught -> window.onerror).
boom() -> probe:panic_now().

%% QUEUED-turn entry (WPORT-7 1c): parks in receive. The first delivered
%% message drives a receive turn on the host microtask, and the panic unwinds
%% into host microtask/timeout dispatch where no caller exists to catch it.
wait_boom() ->
    receive
        _Any -> probe:panic_now()
    end.
