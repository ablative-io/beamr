-module(jit_countdown_loop).
-export([run/0, loop/2]).

%% SLICER GATE (JIT-001): the AOT/demand slicer strips a function's entry Label
%% AND the func_info prelude label that multi-clause dispatch fail-edges target
%% (aot.rs exported_instructions starts at entry+1). So this loop is a TWO-clause
%% function whose second clause is a variable catch-all (no function_clause fail
%% edge to the stripped func_info label), and the self-call is ?MODULE-qualified
%% so erlc emits an external tail call (call_ext_only) that never references the
%% stripped local entry label. Frame / Y-across-call / call_ext_last and
%% local-label CallLast self-recursion end-to-end are deferred to the successor
%% real-erlc-admission brief (continuation call model + slicer label retention).
run() ->
    ?MODULE:loop(5, ok).

loop(0, Acc) ->
    Acc;
loop(N, Acc) ->
    ?MODULE:loop(N - 1, Acc).
