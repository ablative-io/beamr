%% WPORT-6 fixture: the unresolved-import module (D10). Loaded AFTER
%% fetch_chain_c in wall 9, its fetch_chain_c:not_exported/0 import is
%% truly unresolved (module registered, export absent — today's exact
%% report vocabulary); fetch_absent_dep:helper/0 stays deferred with an
%% absent target (missing_dependencies data); fetch_cycle_ping:bounce/1
%% is a deferred import that heals when the target loads later in the
%% same batch.
-module(fetch_unresolved_caller).
-export([run/0, poke/0, wander/0, visit/0]).

run() -> ok.
poke() -> fetch_chain_c:not_exported().
wander() -> fetch_absent_dep:helper().
visit() -> fetch_cycle_ping:bounce(0).
