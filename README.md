# fstsed

*Search and enrich/decorate strings at grep speed*

## Concept

Like [geoipsed](https://github.com/erichutchins/geoipsed) this tool enables in-line enrichment and decoration of text input. geoipsed uses a regular expression to isolate just the strings of interest (IP addresses), but **fstsed** can search for _any_ string. By precomputing a [`fst`](https://blog.burntsushi.net/transducers/) with searchterm and replacement pairs, **fstsed** can turn a database into an ad hoc sed tool.  

## Features
- Search for millions of strings simultaneously with O(1) scaling
- Enrich, decorate, or replace search term with data of your choosing
- Parallel search support (recursive directory walking, respects `.gitignore`)
- JSON search mode to limit searches just within json strings
- High-performance JSON processing: stays at ~200MB/s even with 100k+ keys
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
Usage: fstsed [OPTIONS] -f <FST> [PATH]...

Arguments:
  [PATH]...  Input file(s) or directory(s) to process. Leave empty or use "-" to read from stdin

Options:
  -o, --only-matching        Show only nonempty parts of lines that match
  -C, --color <COLOR>        Use markers to highlight the matching strings [default: auto] [possible values: always, never,
                             auto]
  -f <FST>                   Specify fst db to use in search or to create in build mode
      --build                Build mode. Build a fst from json data instead of querying one. Specify output path with the -f
                             --fst parameter. Only first file input parameter or stdin is used to make the fst
  -k, --key <KEY>            When building a fst, extract the given json field to use as the key in the fst database. Key may
                             also be provided as a jsonpointer, e.g. /obj/array/1/item [default: key]
      --sorted               When building a fst, set this if the keys of input json are already lexicographically sorted. This
                             will make build construction much faster. If this is set but the keys are not sorted, the fst
                             creation will error
  -t, --template <TEMPLATE>  Specify the format of the fstsed match decoration. Field names are enclosed in {}, for example
                             "{field1} any fixed string {field2} & {field3}". Fields may be json keys or jsonpointers
                             {/obj/array/1/item}
  -j, --json                 Json search mode. Fstsed will treat input as json, searching only inside quoted json strings. All
                             strings are deserialized/decoded before json before searching, and all template decorations are
                             properly json-encoded in the output for subsequent processing
  -w, --threads <NUM>        The number of threads to use for searching
  -u, --no-ignore            Do not respect ignore files (.gitignore, .ignore, etc.)
      --hidden               Search hidden files and directories
  -h, --help                 Print help
  -V, --version              Print version

Examples:
  Build an FST database from JSON:
    fstsed --build -f data.fst -k key data.json

  Basic find and replace:
    echo "match" | fstsed -f data.fst

  Use a template for decoration:
    echo "match" | fstsed -f data.fst --template "{key} ({info})"

  JSON search mode (only search inside quoted JSON strings):
    fstsed -f data.fst --json input.json
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

### Performance Scaling

One of the primary advantages of the FST approach is that search performance remains nearly constant regardless of whether you are searching for 10 strings or 100,000 strings.

| Tool | Keys | Throughput (MB/s) | Peak RSS (MB) |
| --- | --- | --- | --- |
| `fstsed --json` | 10 | 203 MB/s | 11 MB |
| `fstsed --json` | 1,000 | 209 MB/s | 11 MB |
| `fstsed --json` | 100,000 | 198 MB/s | 12 MB |
| `rg -F -w` | 10,000 | 66 MB/s | 60 MB |

*Results from 10MB JSON corpus benchmark.*

The JSON search mode is specifically optimized to avoid deserialization overhead for strings without escapes, allowing it to maintain high speeds across extremely large sets of indicators.

### Example: Highlighting Translations

```
; cat burmese.json
{"key":"ဗိုလခုပမူးကီး","translated":"Senior General of Myanmar Army"}

; cat burmese.json | fstsed -f myanmar.fst --build -k key
```

Then, taking the lede from [BBC article](https://www.bbc.com/burmese/burma-57432310) as a test case:

```
; fstsed -f myanmar.fst bbc.txt --template "<{key}> ({translated})"
```

> လနခဲ့တဲ့ ၅ နစက တပမတောကာကယရေးဦးစီးခုပ ရဲ့သကတမးဟာ အကန့အသတမရိတဲ့ သဘောဖစနေလို့ ၆၅ နစကန့သတပီးပငခဲ့တယလို့ **<ဗိုလခုပမူးကီး> (Senior General of Myanmar Army)** မငးအောငလိုငက ပောခဲ့ပီး သူ့အသက းကာ အဲ့ဒီ့ကန့သတခကကို ပယဖကလိုကတဲ့ အတက တပမတောကာကယရေးဦးစီးခုပသကတမးဟာ အကန့အသတမဲ့ ပနဖစသားပတယ။

Even if I can't read any of the Burmese, I still know which key phrase matched, what that phrase means in my native tongue, and where generally the match occurred in the document.


