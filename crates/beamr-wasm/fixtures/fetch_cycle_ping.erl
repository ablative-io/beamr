%% WPORT-6 fixture: one half of the mutual-recursion cycle pair (D10).
-module(fetch_cycle_ping).
-export([bounce/1]).

bounce(0) -> pong_done;
bounce(N) -> fetch_cycle_pong:bounce(N - 1).
