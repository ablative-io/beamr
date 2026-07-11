
Add tests that pin externally meaningful properties through real production
entry points. Include adversarial and failure cases, concurrency repetition when
relevant, and sentinel values that would expose disclosure or data loss. Avoid
tests that merely restate implementation details or pass without exercising the
claimed seam.
