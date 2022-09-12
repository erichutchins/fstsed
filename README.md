# fstsed

*Search and enrich/decorate strings at grep speed*

## Concept

Like [geoipsed](https://github.com/erichutchins/geoipsed) this tool enables in-line enrichment and decoration of text input. `geoipsed` uses a regular expression to isolate just the strings of interest (IP addresses), but **fstsed** can search for _any_ string. By precomputing a [`fst`](https://blog.burntsushi.net/transducers/) with searchterm and replacement pairs, *fstsed* can turn a database into an ad hoc sed tool.  

## Features
- Search for millions of strings simultaneously
- Enrich, decorate, or replace search term with data of your choosing
- JSON search mode to limit searches just within json strings
- Deserialize json strings to search decoded/unescaped strings
- Flexible templating to customize decorations

## Use Cases

Why would I use this? 

| Tool | Find | Enrich |
| --- | --- | --- |
| `grep -f searchterms.txt` | regex and fixed strings | no capability |
| `rg -f searchterms.txt --replace "replacement"` | regex and fixed strings | one replacement string for all searchterms (although it can refer to named capture groups in search patterns) |
| `fstsed` | fixed strings | replacement text *per search term* | 

When would I use this?

- **Indicator database log analysis** - Network defense indicators can take many forms and indicator databases record many dimensions of analyst selected metadata (e.g., attribution, priority, provenance, alerting suitability). Usually, it takes a full blown SIEM to add this enrichment to log analysis, but fstsed gives you a fast ad hoc alternative.

### Note
- Does not match partial strings -- search terms must begin and end with a non-word boundary character
- Longest search terms are matched -- if you have ABC and ABCDE search terms and the text ABCDE, just the ABCDE match occurs, not ABC
- FST files are immutable, any changes require rebuilding the file entirely

## Install

```
# requires fst bin to make databases
cargo install fst-bin

# download and build fstsed
git clone https://github.com/erichutchins/fstsed.git
cd fstsed
export RUSTFLAGS='-C target-cpu=native'
cargo build --release
```

## Examples


