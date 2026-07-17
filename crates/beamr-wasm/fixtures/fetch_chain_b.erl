%% WPORT-6 fixture: middle of the a->b->c dependency chain (D10).
%% Also the deferred-heals dependant of wall 8: when loaded before
%% fetch_chain_c in one batch, its import defers and heals at call time.
-module(fetch_chain_b).
-export([double/1]).

double(X) -> X * fetch_chain_c:base().
