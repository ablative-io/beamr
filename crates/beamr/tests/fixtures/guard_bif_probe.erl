%% Guard-BIF refusal regression fixture (EMB-001 R4), authored fresh for beamr.
%%
%% A minimal typed-operand arithmetic receive loop: the accumulator starts at
%% the integer 0 and each `bump` message (small integer 7) adds 1, which OTP-29
%% erlc emits as a `gc_bif '+'` whose left operand is a typed `{tr, _, {t_integer,
%% _}}` register — the exact instruction shape that refuses at execution when
%% `erlang:'+'/2` resolved to a non-native import (empty BIF registry -> Deferred).
%% A `report` message (small integer 2) returns the accumulator so a populated
%% registry can prove clean arithmetic in the both-directions twin.
-module(guard_bif_probe).
-export([run/0]).

run() ->
    loop(0).

loop(Observed) ->
    receive
        7 ->
            loop(Observed + 1);
        2 ->
            Observed;
        _Other ->
            loop(Observed)
    end.
