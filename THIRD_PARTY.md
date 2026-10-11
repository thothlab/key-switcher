# Данные и код третьих сторон

Здесь — лицензии данных, которые лежат в `data/` и встраиваются в KeySwitcher, и заимствованных списков. Код, взятый из открытых проектов по MIT, атрибутирован в `NOTICE.md` внутри крейта, куда он лёг. Всё в `data/` генерирует `tools/ks-datagen`; источники и числа — в `data/manifest.toml`.

## hunspell `ru_RU` — словарь русского языка

- **Автор:** Alexander I. Lebedev, Copyright (c) 1997-2008. Источник: [LibreOffice dictionaries](https://github.com/LibreOffice/dictionaries/tree/master/ru_RU), коммит `32b006a2c22a4ac7e8ed3f03346f7b3d85a970a4`.
- **Лицензия:** BSD-подобная. Полный текст уведомления — `data/licenses/README_ru_RU.txt` (скопирован без изменений).
- **Модифицирован.** KeySwitcher не распространяет исходные `ru_RU.dic`/`ru_RU.aff`: словарь развёрнут в словоформы по правилам аффиксов, приведён к нижнему регистру, «ё» заменена на «е», формы с цифрами и знаками отброшены, результат сохранён как `fst::Set` (`data/words_ru.fst.gz`). На тех же словоформах обучена символьная языковая модель `data/lm_ru.bin.gz`.
- Имя Alexander I. Lebedev не используется для продвижения KeySwitcher.

```
Copyright (c) 1997-2008, Alexander I. Lebedev

All rights reserved.

Redistribution and use in source and binary forms, with or without
modification, are permitted provided that the following conditions
are met:
* Redistributions of source code must retain the above copyright
  notice, this list of conditions and the following disclaimer.
* Redistributions in binary form must reproduce the above copyright
  notice, this list of conditions and the following disclaimer in the
  documentation and/or other materials provided with the distribution.
* Modified versions must be clearly marked as such.
* The name of Alexander I. Lebedev may not be used to endorse or promote
  products derived from this software without specific prior written
  permission.

THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS "AS IS"
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE
ARE DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT OWNER OR CONTRIBUTORS BE
LIABLE FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR
CONSEQUENTIAL DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF
SUBSTITUTE GOODS OR SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS
INTERRUPTION) HOWEVER CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN
CONTRACT, STRICT LIABILITY, OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE)
ARISING IN ANY WAY OUT OF THE USE OF THIS SOFTWARE, EVEN IF ADVISED OF THE
POSSIBILITY OF SUCH DAMAGE.
```

## hunspell `en_US` — словарь английского языка (SCOWL)

- **Автор:** Kevin Atkinson и участники SCOWL (размер 60), [wordlist.sourceforge.net](http://wordlist.sourceforge.net). Источник: [LibreOffice dictionaries](https://github.com/LibreOffice/dictionaries/tree/master/en), тот же коммит.
- **Лицензия:** пермиссивная — использование, копирование, изменение, распространение и продажа для любой цели при сохранении уведомлений. Полный текст уведомлений всех источников SCOWL — `data/licenses/README_en_US.txt` (скопирован без изменений).
- **Изменения:** словарь развёрнут в словоформы, приведён к нижнему регистру, формы с цифрами и знаками (кроме внутренних `'` и `-`) отброшены; `data/words_en.fst.gz`, языковая модель `data/lm_en.bin.gz`.

## Tatoeba — корпус оценки

- **Что:** выборка предложений на русском и английском, `data/eval/tatoeba_ru.tsv` и `data/eval/tatoeba_en.tsv`. Используется только инструментом замера `ks-eval`, в приложение не встраивается и в обучение не входит.
- **Источник:** [tatoeba.org](https://tatoeba.org), выгрузки per_language (дата выгрузки — в заголовке файла и в `data/manifest.toml`).
- **Лицензия:** [CC BY 2.0 FR](https://creativecommons.org/licenses/by/2.0/fr/). Авторы — участники Tatoeba; автор каждого предложения — по его id: `https://tatoeba.org/sentences/show/<id>` (id — первая колонка файла). Изменения: отобрана часть предложений; текст не изменён.

## RuSwitcher — списки коротких слов

- **Что:** двухбуквенные слова RU и EN и неоднозначные пары (`vs↔мы`, `dj↔во`, `kb↔ли`, `jr↔ок`, `bp↔из`, `ye↔ну`, `ds↔вы`) в `tools/ks-datagen/curated/short_words.tsv` → `data/short_words.tsv` взяты из [RuSwitcher](https://github.com/rashn/RuSwitcher) `macos/Sources/RuSwitcher/ShortWords.swift`. Однобуквенные слова добавлены по собственному разбору.
- **Лицензия:** MIT.

```
MIT License

Copyright (c) 2025 Rashns

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

## Собственные списки

Встроенный список исключённых приложений (`data/excluded_apps.tsv`) и состязательные срезы замера (`data/eval/adversarial.tsv`) составлены для KeySwitcher и сторонних данных не содержат.
