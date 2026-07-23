%% WPORT-7 PROBE-FAILURE leg 1e workload — interleaved out/err console ordering.
%% Fresh-authored, OTP 29 erlc.
%%
%% `gleam_stdlib:println/1`   -> IoStream::Out (console.log under the default sink)
%% `gleam_stdlib:println_error/1` -> IoStream::Err (console.error under default sink)
%% All six writes happen in ONE process turn; the probe records how the platform
%% orders the two streams (Node splits stdout/stderr; a browser console shows one
%% interleaved timeline). Cross-stream order is OUT-OF-CONTRACT (io_sink.rs).
-module(io_probe).
-export([interleave/0]).

interleave() ->
    gleam_stdlib:println(<<"probe-out-1">>),
    gleam_stdlib:println_error(<<"probe-err-1">>),
    gleam_stdlib:println(<<"probe-out-2">>),
    gleam_stdlib:println_error(<<"probe-err-2">>),
    gleam_stdlib:println(<<"probe-out-3">>),
    gleam_stdlib:println_error(<<"probe-err-3">>),
    done.
