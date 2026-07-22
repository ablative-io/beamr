-module(jit_real_function).
-export([run/0, prev/1]).

%% loop/2 is the JIT target. Its recursive clause saves N across an external
%% (?MODULE-qualified) call and threads it into the next turn, so the compiled
%% body carries a stack frame (allocate), a Y register live across a call, a self
%% tail call (call_last), and line instructions -- no arithmetic BIFs, so every
%% opcode stays inside the JIT-002 supported set. It returns an atom (immediate),
%% so the JIT and minimal compositions can be compared across schedulers.
run() ->
    _ = loop(three, none),
    loop(three, none).

loop(zero, Last) ->
    Last;
loop(N, _Last) ->
    loop(?MODULE:prev(N), N).

prev(three) -> two;
prev(two) -> one;
prev(one) -> zero.
