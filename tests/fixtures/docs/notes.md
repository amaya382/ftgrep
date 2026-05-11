---
title: 開発メモ
date: 2026-01-01
category: dev
---

tantivyは高速な全文検索ライブラリです。
Rustで書かれており、Apache Luceneに似たAPIを持ちます。

## 形態素解析

linderaを使うことで日本語の形態素解析が可能です。
IPADICという辞書を使って単語を分割します。
走る、走った、走っていたはすべて「走る」として検索できます。
