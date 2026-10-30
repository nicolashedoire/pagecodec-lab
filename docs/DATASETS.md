# Corpus specification

## Full UCI household electricity corpus

Source: Georges Hebrail and Alice Berard, *Individual Household Electric Power Consumption* (2006), UCI Machine Learning Repository, [DOI 10.24432/C58K54](https://doi.org/10.24432/C58K54), [CC BY 4.0](https://creativecommons.org/licenses/by/4.0/).

The official archive contains `household_power_consumption.txt`: **132,960,755 bytes**, with a header and **2,075,259 records**. Its calendar timestamps span `2006-12-16T17:24:00` through `2010-11-26T21:02:00`. These are naive calendar values; the converter does not invent a timezone.

The source is measured public data, not a process-memory dump. Its derived integers are an explicit application-level representation suitable for a numerical compression experiment. They are not claimed to be the representation originally used by the data collector.

### Exact conversion

[`prepare_household.py`](../scripts/prepare_household.py) parses every row without dropping records. Each numeric row occupies 32 bytes:

| Position | Field | Integer representation |
|---:|---|---|
| 0 | Date + Time | Calendar minutes since the first timestamp |
| 1 | Global_active_power | Value × 1,000 |
| 2 | Global_reactive_power | Value × 1,000 |
| 3 | Voltage | Value × 100 |
| 4 | Global_intensity | Value × 10 |
| 5 | Sub_metering_1 | Value × 1 |
| 6 | Sub_metering_2 | Value × 1 |
| 7 | Sub_metering_3 | Value × 1 |

All words are signed little-endian int32. The scales were fixed before the codec measurements. Every nonmissing decimal is checked for exact integer scaling and an exact inverse numerical conversion using Python `Decimal`. Overflow and collisions with the missing sentinel are rejected. Missing `?` or empty cells become `INT32_MIN` (`-2147483648`); each measurement column contains 25,979 missing cells. The converter rejects invalid dates and sub-minute timestamps.

These files preserve **numerical values, timestamp values and missingness**. They do not preserve decimal spelling such as `1.000` versus `1`, or the distinction between the two textual missing markers. The original text partitions preserve those bytes.

### Layouts and partitions

| Input | Bytes | Description |
|---|---:|---|
| `household-row-i32.bin` | 66,408,288 | Eight words for row 0, then row 1, and so on |
| `household-column-i32.bin` | 66,408,288 | All timestamps, then all active-power values, and so on |
| `household-text-part-01.txt` | 67,108,839 | First complete lines of the original file, including its header |
| `household-text-part-02.txt` | 65,851,916 | Remaining original lines, with no repeated header |

Each numeric layout is **63.331878662109375 MiB**. The text is **126.80 MiB** in total. The comparator's 64 MiB input limit is why the text is partitioned. Concatenating the two text chunks reproduces the source file byte-for-byte; the converter checks its SHA-256. There is no repeated padding or synthetic expansion.

These four files are **one source corpus**, not independent samples. Do not add their sizes together and describe that total as unique observations. Every codec receives identical input bytes within a comparison. Layout conversion and parsing costs are excluded from the codec timing; their cost and application implications would matter in an end-to-end deployment.

### Integrity and download

The [manifest](../manifests/household.json) records the official download URL, archive and source SHA-256, conversion parameters, missing counts, row count, file sizes and derived hashes. The ordinary preparation command verifies that the computed manifest exactly matches the published manifest:

```sh
python3 scripts/prepare_household.py
```

`--archive PATH` reuses a downloaded archive. `--record-manifest` is a maintainer operation for recording a new conversion before measuring; it must not be used to make an unexpected mismatch disappear. Data are fetched from the original source rather than committed as large Git blobs. A future upstream source change will therefore fail verification visibly.

## Earlier experimental corpus

The [initial V2 manifest](../manifests/initial-v2.json) covers 21 inputs totaling 46,403,181 bytes. It includes deterministic synthetic controls and small real inputs. [`prepare_initial.py`](../scripts/prepare_initial.py) reproduces all files and checks each size and hash. The historical manifest prose is retained as recorded, including some French descriptions; the maintained documentation is in English.

The weather int32 subset contains selected measurements only; it is not the original CSV and does not retain its date or categorical weather labels. The REA JavaScript bundle is fetched only as public text to compress: no package installation or code execution is performed. `previous_plot.png` is a fixed, internally generated, already-compressed image used as a corpus fixture; its old French labels are data, not current project documentation.

See [history and limitations](INITIAL_EXPERIMENTS.md) and [source rights](../DATA_LICENSES.md).
