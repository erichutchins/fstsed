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
- [JSON Pointer](https://www.rfc-editor.org/rfc/rfc6901) support both for building and templating
- Zstd compression of input data to minimize size of fst on disk

## Use Cases

Why would I use this? 

| Tool | Find | Enrich |
| --- | --- | --- |
| `grep -f searchterms.txt` | regex and fixed strings | no capability |
| `rg -f searchterms.txt --replace "replacement"` | regex and fixed strings | one replacement string for all searchterms (although it can refer to named capture groups in search patterns) |
| `fstsed` | fixed strings | replacement text **per search term** | 

When would I use this?

- **Indicator database log analysis** - Network defense indicators can take many forms and indicator databases record many dimensions of analyst selected metadata (e.g., attribution, priority, provenance, alerting suitability). Usually, it takes a full blown SIEM to add this enrichment to log analysis, but fstsed gives you a fast ad hoc alternative.

- **Language translation** - Not full translation, but enriching given key words or phrases in place with translation in your native language.

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
Find and replace/decorate text at scale using finite state transducers (fst)

Usage: fstsed [OPTIONS] -f <FST> [FILE]...

Usage: fstsed [OPTIONS] -f <FST> [FILE]...

Arguments:
  [FILE]...  Input file(s) to process (either to search or to use to build the fst). Leave empty or use "-" to read from stdin

Options:
  -o, --only-matching        Show only nonempty parts of lines that match
  -C, --color <COLOR>        Use markers to highlight the matching strings [default: auto] [possible values: always, never, auto]
  -f <FST>                   Specify fst db to use in search or to create in build mode
      --build                Build mode. Build a fst from json data instead of querying one. Specify output path with the -f --fst parameter. Only first file input parameter or stdin is used to make the fst
  -k, --key <KEY>            When building a fst, extract the given json field to use as the key in the fst database. Key may also be provided as a jsonpointer, e.g. /obj/array/1/item [default: key]
      --sorted               When building a fst, set this if the keys of input json are already lexicographically sorted. This will make build construction much faster. If this is set but the keys are not sorted, the fst creation will error
  -t, --template <TEMPLATE>  Specify the format of the fstsed match decoration. Field names are enclosed in {}, for example "{field1} any fixed string {field2} & {field3}". Fields may be json keys or jsonpointers {/obj/array/1/item}
  -j, --json                 Json search mode. Fstsed will treat input as json, searching only inside quoted json strings. All strings are deserialized/decoded before json before searching, and all template decorations are properly json-encoded in the output for subsequent processing
  -h, --help                 Print help
  -V, --version              Print version
```

## Examples

### Volexity IOC database

Let's use Volexity's github repo as an example IOC database. I used a bit of python to convert multiple csvs into a single json file.

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

```
fstsed --build -f volexity.fst -k value volexity.json
# or pipe in from stdin
cat volexity.json | fstsed --build -f volexity.fst -k value 
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

4. **Benchmarks**

Using the volexity fst db on 30k lines of suricata eve json logs from a home network, we can outperform grep for searching. Ripgrep with fixed-string `-F`is the absolute fastest, but there is significant slow down when ensuring matches occur on word boundaries `-w`. Note in this contrived example, there were not matches of the search terms in the data; this is showing the search-only speeds. (hyperfine is ignoring the non-zero exit code because rg and grep did not find any matches)

```shell
; hyperfine -i -w 10 'fstsed -f volexity.fst 30k.log' 'rg -F -w -f volexity.ioc --passthru 30k.log' 'rg -F -f volexity.ioc --passthru 30k.log' 'grep -Fwf volexity.ioc 30k.log' 'fstsed -f volexity.fst --json 30k.log'
Benchmark 1: fstsed -f volexity.fst 30k.log
  Time (mean ± σ):     290.7 ms ±   9.2 ms    [User: 281.4 ms, System: 8.5 ms]
  Range (min … max):   276.6 ms … 303.2 ms    10 runs
 
Benchmark 2: rg -F -w -f volexity.ioc --passthru 30k.log
  Time (mean ± σ):     343.9 ms ±   9.6 ms    [User: 321.6 ms, System: 20.5 ms]
  Range (min … max):   335.5 ms … 364.9 ms    10 runs
 
  Warning: Ignoring non-zero exit code.
 
Benchmark 3: rg -F -f volexity.ioc --passthru 30k.log
  Time (mean ± σ):     160.0 ms ±   9.0 ms    [User: 132.7 ms, System: 26.3 ms]
  Range (min … max):   144.9 ms … 180.6 ms    18 runs
 
  Warning: Ignoring non-zero exit code.
 
Benchmark 4: grep -Fwf volexity.ioc 30k.log
  Time (mean ± σ):     438.7 ms ±  21.2 ms    [User: 428.9 ms, System: 9.1 ms]
  Range (min … max):   414.0 ms … 478.2 ms    10 runs
 
  Warning: Ignoring non-zero exit code.
 
Benchmark 5: fstsed -f volexity.fst --json 30k.log
  Time (mean ± σ):     543.8 ms ±  17.1 ms    [User: 532.8 ms, System: 8.0 ms]
  Range (min … max):   522.9 ms … 579.8 ms    10 runs
 
Summary
  'rg -F -f volexity.ioc --passthru 30k.log' ran
    1.82 ± 0.12 times faster than 'fstsed -f volexity.fst 30k.log'
    2.15 ± 0.13 times faster than 'rg -F -w -f volexity.ioc --passthru 30k.log'
    2.74 ± 0.20 times faster than 'grep -Fwf volexity.ioc 30k.log'
    3.40 ± 0.22 times faster than 'fstsed -f volexity.fst --json 30k.log'
```

### Example: Highlighting Translations

```
; cat burmese.json
{"key":"ဗိုလခုပမူးကီး","translated":"Senior General of Myanmar Army"}

; cat burmese.json fstsed -f myanmar.fst --build -k key
```

Then, taking the lede from [BBC article](https://www.bbc.com/burmese/burma-57432310) as a test case:

```
; fstsed -f myanmar.fst bbc.txt --template "<{key}> ({translated})"
```

> လနခဲ့တဲ့ ၅ နစက တပမတောကာကယရေးဦးစီးခုပ ရဲ့သကတမးဟာ အကန့အသတမရိတဲ့ သဘောဖစနေလို့ ၆၅ နစကန့သတပီးပငခဲ့တယလို့ **<ဗိုလခုပမူးကီး> (Senior General of Myanmar Army)** မငးအောငလိုငက ပောခဲ့ပီး သူ့အသက းကာ အဲ့ဒီ့ကန့သတခကကို ပယဖကလိုကတဲ့ အတက တပမတောကာကယရေးဦးစီးခုပသကတမးဟာ အကန့အသတမဲ့ ပနဖစသားပတယ။

Even if I can't read any of the Burmese, I still know which key phrase matched, what that phrase means in my native tongue, and where generally the match occurred in the document.


