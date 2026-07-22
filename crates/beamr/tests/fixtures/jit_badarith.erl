-module(jit_badarith).
-export([driver/1, f/1]).

%% SLICER GATE (JIT-001): as with the countdown fixture, calls are ?MODULE-
%% qualified (external, call_ext_only) to dodge the entry-label strip. driver/1
%% tail-calls f/1 so f/1 heats at the external edge; f/1's `X - 1` is a body-
%% position {f,0} arithmetic Bif that routes to deopt (R3 BIF NO-FAIL RULING).
%% A non-integer X provokes a badarith through the native->deopt->interpreter
%% path, which must be observably equal to the interpreter-only composition.
driver(X) ->
    ?MODULE:f(X).

f(X) ->
    X - 1.
