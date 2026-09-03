defmodule T1Task do
  def main do
    t = Task.async(fn -> 21 * 2 end)
    Task.await(t)
  end
end
