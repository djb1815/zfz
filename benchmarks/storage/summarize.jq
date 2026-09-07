def percentile($p): sort | .[((length - 1) * $p / 100 | floor)];
.results[] |
  .command as $command |
  .times as $times |
  [$command,
   ($times | min),
   ($times | percentile(50)),
   ($times | percentile(95)),
   ($times | percentile(99)),
   ($times | max)] |
  @tsv
