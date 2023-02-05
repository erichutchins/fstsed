# fstsed

*Search and enrich/decorate strings at grep speed*

## Concept

Like [geoipsed](https://github.com/erichutchins/geoipsed) this tool enables in-line enrichment and decoration of text input. geoipsed uses a regular expression to isolate just the strings of interest (IP addresses), but **fstsed** can search for _any_ string. By precomputing a [`fst`](https://blog.burntsushi.net/transducers/) with searchterm and replacement pairs, **fstsed** can turn a database into an ad hoc sed tool.  

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
| `fstsed` | fixed strings | replacement text **per search term** | 

When would I use this?

- **Indicator database log analysis** - Network defense indicators can take many forms and indicator databases record many dimensions of analyst selected metadata (e.g., attribution, priority, provenance, alerting suitability). Usually, it takes a full blown SIEM to add this enrichment to log analysis, but fstsed gives you a fast ad hoc alternative.

### Note
- Does not match partial strings -- search terms must begin and end with a non-word boundary character
- Longest search terms are matched -- if you have ABC and ABCDE search terms and the text ABCDE, just the ABCDE match occurs, not ABC
- FST files are immutable, any changes require rebuilding the file entirely

## Install

```
# optional, install fst bin to view fst databases
cargo install fst-bin

# download and build fstsed
git clone https://github.com/erichutchins/fstsed.git
cd fstsed
export RUSTFLAGS='-C target-cpu=native'
cargo build --release
```

## Usage 

```
Usage: fstsed [OPTIONS] -f <FST> [FILE]...

Arguments:
  [FILE]...  Input file(s) to process. Leave empty or use "-" to read from stdin

Options:
  -o, --only-matching        Show only nonempty parts of lines that match
  -C, --color <COLOR>        Use markers to highlight the matching strings [default: auto] [possible values: always, never, auto]
  -f <FST>                   Specify fst db to use
      --build                Build a fst from json data instead of querying one. Specify output path with the -f --fst parameter
  -k, --key <KEY>            When building, extract the given field to use as the key in the fst database [default: key]
  -t, --template <TEMPLATE>  Specify the format of the fstsed match decoration. Field names are enclosed in {}, for example "{field1} any fixed string {field2} & {field3}"
  -j, --json                 Specify json input. Fstsed will only search inside quoted json strings additionally deserialize/decode json strings before searching. Ensures all template decorations are properly encoded for subsequent json processing
  -h, --help                 Print help
  -V, --version              Print version
```

## Examples

### Volexity IOC database

Let's use Volexity github repo as an example IOC database. I used a bit of python to convert multiple csvs into a single json file.

1. **Get the data**
```
git clone https://github.com/volexity/threat-intel.git

ipython
```

```python
In [1]: import pandas as pd
In [2]: import glob
In [3]: csvs = glob.glob("**/*.csv", recursive=True)
In [4]: def conv(csv):
   ...:     df = pd.read_csv(csv)
   ...:     df["path"] = csv
   ...:     return df.to_json(orient="records", lines=True)
   ...: 
In [5]: iocjson = "\n".join(map(conv, csvs))
In [6]: with open("volexity.json", "w") as f:
   ...:     f.write(iocjson)
```

2. **Build FST database**

2.1. **Using fstsed**

```
cat volexity.json | fstsed -f volexity.fst -k value
```

2.2. **Using fst itself**

The way we encode the database is to join key and value with a null byte. The jq function extracts the `.value` of the indicator, prints a null `\u0000`, and then prints the whole record back out as json. Then the fst bin utility does the work of making the transducer itself.

```
cat volexity.json | jq -r '.value + "\u0000" + tojson' | fst set - volexity.fst
```

3. **Now we can play**

Basic find and replace but because we didn't specify a template, the replace value is the entire json record.

```
echo "test of avsvmcloud.com metadata" | fstsed -f volexity.fst 
test of <avsvmcloud.com|{"value":"avsvmcloud.com","type":"hostname","notes":null,"path":"2020/2020-12-14 - DarkHalo Leverages SolarWinds Compromise to Breach Organizations/indicators/indicators.csv"}> metadata
```

Specify a template, which can include prose/fixed strings as well as and top level key in json record in the fst.

```
echo "test of avsvmcloud.com metadata" | fstsed -f volexity.fst --template "{key} (a {type} from {path} report)"
test of avsvmcloud.com (a hostname from 2020/2020-12-14 - DarkHalo Leverages SolarWinds Compromise to Breach Organizations/indicators/indicators.csv report) metadata
```

