# Flexo

Successor of cpcache. Still a work in progress – Do not use unless you intend to waste your time.

## long term goal
Provide the same functionality as cpcache, with the following advantages:
* More lightweight
* Easier to deploy
* More robust

The first two points are achieved by using Rust instead of Elixir. Robustness is achieved by using a
battletested and well known library: curl.
