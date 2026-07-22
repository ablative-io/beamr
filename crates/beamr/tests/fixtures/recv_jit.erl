-module(recv_jit).
-export([run/0, loop/1, echo/0]).

%% run/0 pre-loads its own mailbox, then loops calling echo/0 through the external
%% (?MODULE) edge so echo/0 HEATS and compiles through the demand path.
run() ->
    self() ! a,
    self() ! b,
    self() ! c,
    self() ! d,
    self() ! e,
    ?MODULE:loop(5).

loop(0) -> ok;
loop(N) ->
    _ = ?MODULE:echo(),
    ?MODULE:loop(N - 1).

%% A single blocking receive: loop_rec (pure peek), remove_message (the consume,
%% on the matched path that RETURNS), wait (pure park). No deopt-capable op is
%% reachable after the consume, so the CFG-sensitive guard ADMITS it.
echo() ->
    receive
        M -> M
    end.
