# Extract the shipped `run:` blocks OUT OF ci.yml so the controls exercise the
# bytes that ship, not a hand-copy. A control that runs a retyped copy of the
# code proves the retyped copy works.
#
# Usage: EV=<dir> YML=<path> ruby extract.rb

ev  = ENV.fetch("EV")
yml = ENV.fetch("YML")

lines = File.readlines(yml, chomp: true)

# Pull the literal-block scalar belonging to the step named `step_name`.
def block_for(lines, step_name)
  i = lines.index { |l| l.strip == "- name: #{step_name}" }
  raise "step not found: #{step_name}" if i.nil?

  # Walk forward to this step's `run: |`
  j = i
  j += 1 while j < lines.length && lines[j].strip != "run: |"
  raise "no `run: |` for step: #{step_name}" if j >= lines.length

  run_indent = lines[j][/\A */].length
  body = []
  k = j + 1
  while k < lines.length
    l = lines[k]
    break if !l.strip.empty? && l[/\A */].length <= run_indent
    body << l
    k += 1
  end

  # Dedent by the block's own minimum indent over NON-BLANK lines only —
  # blank lines carry no indent and would floor the minimum at zero.
  non_blank = body.reject { |l| l.strip.empty? }
  indent = non_blank.map { |l| l[/\A */].length }.min
  out = body.map { |l| l.strip.empty? ? "" : l[indent..] }

  # Drop the blank separator line(s) between this step and the next, so the
  # artifact's sha is a function of the script and nothing else.
  out.pop while !out.empty? && out.last.empty?
  out.join("\n") + "\n"
end

{ "canon.sh" => "Run the canon", "verdict.sh" => "Verdict" }.each do |file, step|
  out = File.join(ev, file)
  File.write(out, block_for(lines, step))
  warn "extracted #{step.inspect} -> #{out}"
end
