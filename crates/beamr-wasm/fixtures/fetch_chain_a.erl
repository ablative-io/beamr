%% WPORT-6 fixture: head of the a->b->c dependency chain (D10); the
%% end-to-end wall spawns fetch_chain_a:run/0 and observes 42.
-module(fetch_chain_a).
-export([run/0]).

run() -> fetch_chain_b:double(20) + 2.
