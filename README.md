# seaf-share

[![ci badge](https://ci.codeberg.org/api/badges/14311/status.svg)](https://ci.codeberg.org/repos/14311)
[![crates.io badge](https://img.shields.io/crates/v/seaf-share)](https://crates.io/crates/seaf-share)

Seafile share CLI tool

* List files in a share
* Download files (recursively) in a share

## Install

```console
cargo install seaf-share
```

## Examples

```console
$ seaf-share list https://cloud.tsinghua.edu.cn/d/df2ff6121f3f4edfaff4/
+---------------------------------------------------+-----------+---------------------------+
| Name                                              | Size      | Last Modified             |
+---------------------------------------------------+-----------+---------------------------+
| Reference Only_2024 Information/                  | N/A       | 2025-01-13T06:09:58+00:00 |
+---------------------------------------------------+-----------+---------------------------+
| 2025 Tsinghua SIGS Global Summer School_Flyer.pdf | 22.6 MiB  | 2025-01-13T06:12:16+00:00 |
+---------------------------------------------------+-----------+---------------------------+
| Recap_Tsinghua SIGS Global Summer School.mp4      | 495.8 MiB | 2024-12-17T07:21:00+00:00 |
+---------------------------------------------------+-----------+---------------------------+
```

```console
$ seaf-share download -r https://cloud.tsinghua.edu.cn/d/df2ff6121f3f4edfaff4/
```
