-module(frameless).
-export([run/0, loop/2]).
run() -> ?MODULE:loop(three, won).
loop(zero, Acc) -> Acc;
loop(three, Acc) -> ?MODULE:loop(two, Acc);
loop(two, Acc) -> ?MODULE:loop(one, Acc);
loop(one, Acc) -> ?MODULE:loop(zero, Acc).
