//! Счётчики замера и их таблицы в Markdown.

use std::fmt::Write as _;

use crate::Variant;

/// `hit` из `n`: ложные исправления из токенов или верные исправления из ошибок.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub n: u64,
    pub hit: u64,
}

impl Counts {
    pub fn add(&mut self, hit: bool) {
        self.n += 1;
        self.hit += u64::from(hit);
    }

    pub fn per_10k(&self) -> f64 {
        self.share() * 10_000.0
    }

    pub fn share(&self) -> f64 {
        if self.n == 0 {
            0.0
        } else {
            self.hit as f64 / self.n as f64
        }
    }
}

/// Корзина длины слова в буквах: 1, 2, 3, 4+ → индекс 0..=3.
pub fn len_bucket(letters: usize) -> usize {
    letters.clamp(1, 4) - 1
}

/// Ложные исправления на живом тексте.
#[derive(Debug, Default)]
pub struct LiveRow {
    pub all: Counts,
    pub by_len: [Counts; 4],
}

/// Полнота на синтетических ошибках раскладки.
#[derive(Debug, Default)]
pub struct RecallRow {
    pub dict: Counts,
    /// Словарные от двух букв: цель ≥ 97% (однобуквенные меряются в потоке фразы).
    pub dict2: Counts,
    pub oov: Counts,
    /// Исправлено, но не в задуманный текст (регистр, знак, другое слово).
    pub wrong: u64,
    /// Слова, которые не набрать в одной из раскладок: в полноту не входят.
    pub skipped: u64,
    pub by_len: [Counts; 4],
    pub by_variant: [Counts; 5],
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecallOutcome {
    Fixed,
    Wrong,
    Missed,
}

/// Поток фразы: фраза набрана целиком в неверной раскладке, после исправления раскладка
/// переключена. Однобуквенные — все и отдельно набранные ещё в неверной раскладке.
#[derive(Debug, Default)]
pub struct FlowRow {
    pub phrases: Counts,
    pub one_letter: Counts,
    pub one_letter_wrong_layout: Counts,
    /// Фразы, которые не набрать в одной из раскладок.
    pub skipped: u64,
}

/// Результаты замера по срезам в порядке появления.
#[derive(Debug, Default)]
pub struct Tally {
    live: Vec<(String, LiveRow)>,
    recall: Vec<(String, RecallRow)>,
    adversarial: Vec<(String, Counts)>,
    flow: Vec<(String, FlowRow)>,
}

fn row<'a, T: Default>(rows: &'a mut Vec<(String, T)>, slice: &str) -> &'a mut T {
    let pos = match rows.iter().position(|(s, _)| s == slice) {
        Some(pos) => pos,
        None => {
            rows.push((slice.to_owned(), T::default()));
            rows.len() - 1
        }
    };
    &mut rows[pos].1
}

fn find<'a, T>(rows: &'a [(String, T)], slice: &str) -> Option<&'a T> {
    rows.iter().find(|(s, _)| s == slice).map(|(_, r)| r)
}

impl Tally {
    pub fn live(&mut self, slice: &str, letters: usize, corrected: bool) {
        let r = row(&mut self.live, slice);
        r.all.add(corrected);
        r.by_len[len_bucket(letters)].add(corrected);
    }

    pub fn recall(
        &mut self,
        slice: &str,
        in_dict: bool,
        letters: usize,
        variant: Variant,
        outcome: RecallOutcome,
    ) {
        let r = row(&mut self.recall, slice);
        let fixed = outcome == RecallOutcome::Fixed;
        if in_dict {
            r.dict.add(fixed);
        } else {
            r.oov.add(fixed);
        }
        if in_dict && letters >= 2 {
            r.dict2.add(fixed);
        }
        r.wrong += u64::from(outcome == RecallOutcome::Wrong);
        r.by_len[len_bucket(letters)].add(fixed);
        r.by_variant[variant as usize].add(fixed);
    }

    /// Слово, которое не удалось набрать в неверной раскладке.
    pub fn recall_skip(&mut self, slice: &str) {
        row(&mut self.recall, slice).skipped += 1;
    }

    /// Фраза потока: верна ли целиком; однобуквенные — (набрано в неверной раскладке, верно).
    pub fn flow(&mut self, slice: &str, phrase_ok: bool, one_letter: &[(bool, bool)]) {
        let r = row(&mut self.flow, slice);
        r.phrases.add(phrase_ok);
        for &(wrong_layout, ok) in one_letter {
            r.one_letter.add(ok);
            if wrong_layout {
                r.one_letter_wrong_layout.add(ok);
            }
        }
    }

    pub fn flow_skip(&mut self, slice: &str) {
        row(&mut self.flow, slice).skipped += 1;
    }

    pub fn flow_row(&self, slice: &str) -> Option<&FlowRow> {
        find(&self.flow, slice)
    }

    pub fn flow_rows(&self) -> impl Iterator<Item = (&str, &FlowRow)> {
        self.flow.iter().map(|(s, r)| (s.as_str(), r))
    }

    /// Худшая полнота по словарным словам от двух букв среди срезов, где они есть.
    pub fn dict_recall_from2(&self) -> f64 {
        self.recall
            .iter()
            .filter(|(_, r)| r.dict2.n > 0)
            .map(|(_, r)| r.dict2.share())
            .fold(1.0, f64::min)
    }

    /// Худшая доля верных фраз в потоке.
    pub fn flow_phrases(&self) -> f64 {
        self.flow
            .iter()
            .filter(|(_, r)| r.phrases.n > 0)
            .map(|(_, r)| r.phrases.share())
            .fold(1.0, f64::min)
    }

    pub fn adversarial(&mut self, slice: &str, corrected: bool) {
        row(&mut self.adversarial, slice).add(corrected);
    }

    pub fn live_row(&self, slice: &str) -> Option<&LiveRow> {
        find(&self.live, slice)
    }

    pub fn recall_row(&self, slice: &str) -> Option<&RecallRow> {
        find(&self.recall, slice)
    }

    pub fn adversarial_row(&self, slice: &str) -> Option<&Counts> {
        find(&self.adversarial, slice)
    }

    pub fn live_rows(&self) -> impl Iterator<Item = (&str, &LiveRow)> {
        self.live.iter().map(|(s, r)| (s.as_str(), r))
    }

    pub fn recall_rows(&self) -> impl Iterator<Item = (&str, &RecallRow)> {
        self.recall.iter().map(|(s, r)| (s.as_str(), r))
    }

    pub fn adversarial_rows(&self) -> impl Iterator<Item = (&str, &Counts)> {
        self.adversarial.iter().map(|(s, r)| (s.as_str(), r))
    }

    /// Худший срез живого текста по FP на 10k и худшая полнота по словарным словам
    /// среди срезов, где словарные слова есть.
    pub fn headline(&self) -> (f64, f64) {
        let fp = self
            .live
            .iter()
            .map(|(_, r)| r.all.per_10k())
            .fold(0.0, f64::max);
        let recall = self
            .recall
            .iter()
            .filter(|(_, r)| r.dict.n > 0)
            .map(|(_, r)| r.dict.share())
            .fold(1.0, f64::min);
        (fp, recall)
    }

    pub fn render_markdown(&self) -> String {
        let mut out = String::new();
        let per10k = |c: &Counts| {
            if c.n == 0 {
                "—".to_owned()
            } else {
                format!("{:.1} ({})", c.per_10k(), c.n)
            }
        };
        let pct = |c: &Counts| {
            if c.n == 0 {
                "—".to_owned()
            } else {
                format!("{:.1}% ({})", c.share() * 100.0, c.n)
            }
        };
        let _ = writeln!(
            out,
            "#### Ложные исправления на живом тексте\n\n\
             FP на 10 000 токенов; в скобках — число токенов в срезе.\n\n\
             | Срез | Токенов | FP | FP/10k | 1 буква | 2 | 3 | 4+ |\n|---|---|---|---|---|---|---|---|"
        );
        for (slice, r) in &self.live {
            let _ = writeln!(
                out,
                "| {slice} | {} | {} | {:.1} | {} | {} | {} | {} |",
                r.all.n,
                r.all.hit,
                r.all.per_10k(),
                per10k(&r.by_len[0]),
                per10k(&r.by_len[1]),
                per10k(&r.by_len[2]),
                per10k(&r.by_len[3]),
            );
        }
        let _ = writeln!(
            out,
            "\n#### Полнота на синтетических ошибках раскладки\n\n\
             Доля ошибок, исправленных точно в задуманный текст; в скобках — число ошибок. \
             «Не набрать» — слова, которых нет в таблице одной из раскладок; в полноту не входят.\n\n\
             | Срез | Словарные | Словарные от 2 букв | Внесловарные | Исправлено неверно | Не набрать | 1 буква | 2 | 3 | 4+ |\n\
             |---|---|---|---|---|---|---|---|---|---|"
        );
        for (slice, r) in &self.recall {
            let _ = writeln!(
                out,
                "| {slice} | {} | {} | {} | {} | {} | {} | {} | {} | {} |",
                pct(&r.dict),
                pct(&r.dict2),
                pct(&r.oov),
                r.wrong,
                r.skipped,
                pct(&r.by_len[0]),
                pct(&r.by_len[1]),
                pct(&r.by_len[2]),
                pct(&r.by_len[3]),
            );
        }
        let _ = writeln!(
            out,
            "\n#### Поток фразы\n\n\
             Фраза набрана целиком в неверной раскладке; после первого исправления раскладка \
             переключается, дальше слова набираются верно. Фраза верна, если в итоге на экране \
             задуманный текст. Однобуквенные — все и отдельно те, что набраны ещё в неверной раскладке.\n\n\
             | Срез | Верных фраз | 1 буква | 1 буква в неверной раскладке | Не набрать |\n|---|---|---|---|---|"
        );
        for (slice, r) in &self.flow {
            let _ = writeln!(
                out,
                "| {slice} | {} | {} | {} | {} |",
                pct(&r.phrases),
                pct(&r.one_letter),
                pct(&r.one_letter_wrong_layout),
                r.skipped
            );
        }
        let _ = write!(out, "\n#### Полнота по вариантам набора\n\n| Срез |");
        for v in Variant::ALL {
            let _ = write!(out, " {} |", v.label());
        }
        let _ = writeln!(out, "\n|---|---|---|---|---|---|");
        for (slice, r) in &self.recall {
            let _ = write!(out, "| {slice} |");
            for c in &r.by_variant {
                let _ = write!(out, " {} |", pct(c));
            }
            let _ = writeln!(out);
        }
        let _ = writeln!(
            out,
            "\n#### Состязательные срезы\n\n\
             Токены набраны правильно, каждый отдельно; любое исправление — ложное.\n\n\
             | Срез · язык · раскладка | Токенов | Исправлено | Доля |\n|---|---|---|---|"
        );
        for (slice, c) in &self.adversarial {
            let _ = writeln!(
                out,
                "| {slice} | {} | {} | {:.2}% |",
                c.n,
                c.hit,
                c.share() * 100.0
            );
        }
        out
    }
}
