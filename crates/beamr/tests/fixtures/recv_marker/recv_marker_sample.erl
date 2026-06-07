-module(recv_marker_sample).
-export([recv_once/0, recv_ref/1]).

recv_once() ->
    Ref = make_ref(),
    recv_ref(Ref).

recv_ref(Ref) ->
    receive
        {Ref, Msg} -> Msg
    after 0 ->
        timeout
    end.
