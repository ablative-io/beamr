defmodule T1Process do
  def main do
    parent = self()
    spawn(fn -> send(parent, {:pong, self()}) end)
    receive do
      {:pong, _pid} -> :ok
    after
      1000 -> :timeout
    end
  end
end
