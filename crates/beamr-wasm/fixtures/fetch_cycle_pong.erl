%% WPORT-6 fixture: other half of the mutual-recursion cycle pair (D10).
-module(fetch_cycle_pong).
-export([bounce/1]).

bounce(0) -> ping_done;
bounce(N) -> fetch_cycle_ping:bounce(N - 1).
