# Russian glossary (ru, Русский)

Player addressed as "вы" (lower case). Unit, map and planet names stay in Latin script as the
game data shows them (Peewee, Kbot Lab, Empyrrean); faction names stay Arm / Core (ARM / CORE),
anchored with a noun where Russian needs gender (силы Core, армия Arm).

| English | Russian |
|---|---|
| Commander | Командир |
| metal / energy | металл / энергия (counts: "{n} металла", "{n} энергии") |
| unit | юнит (counts: "Юниты: {n}", "Уничтожено юнитов: {n}") |
| building / structure | здание / сооружение |
| factory | завод |
| constructor, builder | строитель |
| kbot | кбот |
| vehicle | техника |
| aircraft | авиация (самолёт) |
| hovercraft | СВП (судно на воздушной подушке) |
| ship / submarine / navy | корабль / подлодка / флот |
| wreck | обломки |
| reclaim | переработка, переработать |
| repair / mend | ремонт / починить |
| guard / patrol / attack | охрана / патруль / атака |
| fight (attack-move) | бой (движение с атакой) |
| capture / resurrect | захват / воскрешение |
| load / unload (transport) | погрузка / выгрузка |
| fire orders | режим огня: не стрелять / ответный огонь / свободный огонь |
| move orders | режим движения: держать позицию / манёвр / погоня |
| self-destruct | самоуничтожение (SELF-D: ПОДРЫВ) |
| cloak / decloak | маскировка / снять маскировку |
| radar / sonar / jammer | радар / сонар / постановщик помех |
| line of sight, sight | обзор |
| fog of war | туман войны |
| mapped / unmapped | открыта / скрыта |
| skirmish | схватка |
| campaign / mission / briefing | кампания / миссия / брифинг |
| scenario / conquest | сценарий / завоевание |
| multiplayer / lobby | сетевая игра / лобби |
| host / join | хост, создать / подключиться |
| spectator | зритель |
| replay | повтор |
| highlights | лучшие моменты |
| saved game / autosave | сохранение / автосохранение |
| map editor | редактор карт |
| tile / feature (map) | тайл / объект |
| tile set | набор тайлов |
| this map (stands in for {planet}) | этой карты ("Для {planet}: {n}") |
| mod | мод |
| archives / encyclopaedia | архив / энциклопедия |
| timeline / credits | хроника / титры |
| jukebox | плеер |
| launcher | лаунчер |
| settings, options | настройки |
| key bindings | клавиши, управление |
| nuke / anti-nuke | ядерная ракета / ПРО |
| stockpile | запас |
| queue | очередь |
| control group, squad | группа, отряд |
| ally / enemy / team | союзник / враг, противник / команда |
| victory / defeat | победа / поражение |
| computer player (AI) | компьютерный игрок (ИИ) |
| Easy / Normal / Hard (AI) | Лёгкий / Обычный / Сложный (EASY/MEDIUM/HARD: ЛЁГКИЙ/СРЕДНИЙ/СЛОЖНЫЙ, masculine for уровень) |
| D-gun | Д-пушка |
| the Galactic War | Галактическая война |
| patterning (lore) | оцифровка (сознания) |
| ranks | Новобранец / Ветеран / Закалённый / Элита |
| extractor | экстрактор (металла) |
| storage / income / spending | хранилище / доход / расход |
| health / health bars | здоровье / полосы здоровья |
| idle | без дела, незанятый |
| kills | уничтожено |
| build power | мощность (строительства) |
| build time | время постройки (ВРЕМЯ ПОСТРОЙКИ) |
| Perspective (first-person mode) | Перспектива |
| photo mode | фоторежим |
| mouse (controls column) | Щелчок, Щелчок ПКМ, Двойной щелчок, Протяжка, Протяжка ПКМ / СКМ, Колесо мыши |
| build tabs | РЕСУРСЫ / ЗАВОДЫ / ОБОРОНА / СЕНСОРЫ / ПРОЧЕЕ |
| cheats / cheat codes | читы / чит-коды |
| resign | сдаться (REALLY RESIGN? ТОЧНО СДАТЬСЯ?) |
| outcome titles | ПОБЕДА / ПОРАЖЕНИЕ / МИССИЯ ВЫПОЛНЕНА / МИССИЯ ПРОВАЛЕНА; OVERRUN (endless siege lost): ОБОРОНА ПАЛА |
| graphics levels (masculine, уровень) | АВТО / НИЗКИЙ / СРЕДНИЙ / ВЫСОКИЙ / УЛЬТРА / СВОЙ (CUSTOM; the resolution's "Custom" is Своё) |
| conquest opposition (masculine, противник) | ЩАДЯЩИЙ / СТАНДАРТНЫЙ / СЛОЖНЫЙ; galaxy size МАЛЫЙ / СРЕДНИЙ / БОЛЬШОЙ |
| scenario kinds (tags) | ВЫЖИВАНИЕ / БЕЗ КОНЦА / УДЕРЖАНИЕ / ЭСКОРТ / УДАР / НА ВРЕМЯ / ГОНКА / БИТВА |
| scenario goal phrases | accusative objects in natural frames: "Уничтожить {what}", "Не потерять {what}", "Довести {what} до точки" (своего Командира, любой свой юнит, вражеского Командира, все юниты врага, армию врага, все свои юниты) |
| ship's telegraph | ПОЛНЫЙ ХОД / СРЕДНИЙ ХОД / ЗАДНИЙ ХОД / СТОП |
| gamepad | Левый / Правый стик, Нажатие левого стика, Крестовина, LB (удерживать) |
| players panel / stats headers | ИГРОК / СТОРОНА / КОМАНДА / СТАТУС; ДОБЫТО МЕТАЛЛА, ВЫРАБОТАНО ЭНЕРГИИ, ПОТРАЧЕНО МЕТАЛЛА / ЭНЕРГИИ |
| saved-game list | tag СОХР. (beside ПОВТОР), ЗАГРУЗИТЬ / СМОТРЕТЬ, "время битвы {length}", ages: только что, {n} мин / ч / дн. назад |

Width-driven choices (measured in Bahnschrift against the fixed button sizes in the code):
SAVE → СОХР. (the editor's 84 px button: СОХРАНИТЬ measures 82 px with the button's tracking)
and SAVED → СОХР. too (the tag's slot is 62 px, about 50 px of text: СОХРАНЕНИЕ is 74 px);
the stats table's four resource headers are two lines each ("ДОБЫТО\nМЕТАЛЛА"): on one line
the table is ~1040 px in a 720 px panel, on two ~790 px; CONTINUE CONQUEST →
К ЗАВОЕВАНИЮ; DISCARD CHANGES → НЕ СОХРАНЯТЬ; KEEP EDITING → ОСТАТЬСЯ; DESTROY (cheats) →
ВЗОРВАТЬ; scenario goal panel (250 px): "Удерживать участок", "ещё {n}", "через {clock}".
Counts never meet a declined noun: "Юнитов: {n}", "{n} металла", "{minutes}-я минута", "пикс.".
| Recycle Bin (Windows' own name) | Корзина |
| crash report | отчёт о сбое |
| changelog / WHAT'S NEW | список изменений / ЧТО НОВОГО |
| update / UPDATE NOW / INSTALL NOW | обновление, обновить / ОБНОВИТЬ СЕЙЧАС / УСТАНОВИТЬ СЕЙЧАС (as CHECK NOW: ПРОВЕРИТЬ СЕЙЧАС; fits the buttons' 170 px) |
| header pill UPDATE v… / UPDATING TO v… (UPDATED TO v…) | ОБНОВЛЕНИЕ v… / ОБНОВЛЕНИЕ ДО v… (ОБНОВЛЕНО ДО v…) |
| release (a published version) | релиз |
| Setup (TA Reborn's installer), setup, installer | установщик ("установлена не через установщик") |
| the download (the file) / CHECKING THE DOWNLOAD… | скачанный файл / ПРОВЕРКА ЗАГРУЗКИ… |
| SHA-256 checksum list | список контрольных сумм SHA-256 |
| GNU General Public License | Стандартная общественная лицензия GNU (the FSF's Russian name); free software: свободное программное обеспечение |
| the Play page | страница «Играть» (as «Моды») |
| benchmark | бенчмарк (БЕНЧМАРК: the launcher's rail leaves ~95 px beside its running badge, too little for «тест производительности»); RUN BENCHMARK / RUN AGAIN → НАЧАТЬ БЕНЧМАРК / НАЧАТЬ СНОВА (the hero button is 180 px when narrow) |
| a benchmark run | прогон (ПРОШЛЫЕ ПРОГОНЫ, THE RUN IN TIME → ХОД ПРОГОНА); EACH BATTLE → ПО БИТВАМ |
| settings test | тест настроек |
| SINCE {when} (the change since the run before; {when} = "2 ДН. НАЗАД", "ВЧЕРА") | ПРОШЛЫЙ ПРОГОН: {when} |
| much the same (a change under 1%) / The run before scored {score} | почти так же / Оценка прошлого прогона: {score} |
| OPEN REPORT FILE (opens the run's JSON) | ОТКРЫТЬ ФАЙЛ ОТЧЁТА |
| its slowest frames held {fps} a second / near enough to stay smooth | "Самые медленные кадры в тесте — {fps} FPS, выше цели в {target}" / "достаточно близко к цели в {target}, чтобы картинка оставалась плавной" |
| score(s) / points | оценка, оценки / баллы ("набирает 10 000 баллов"); SCORED {when} → ИЗМЕРЕНО {when} (ИЗМЕРЕНО 19 МИН НАЗАД); CLASS → КЛАСС |
| reference computer | эталонный компьютер |
| graphics card / processor / memory / driver / screen | видеокарта / процессор / память / драйвер / экран |
| GPU- / CPU-bound ("sets the pace") | всё упирается в видеокарту / в процессор; balanced: идут вровень |
| FPS, frame rate | FPS after a count ("{fps} FPS", never a declined кадров after a placeholder); legend: частота кадров; literal numbers spelled out ("60 кадров в секунду") |
| 1% LOW / 0.1% LOW | 1% МИН. / 0.1% МИН. (in a line: "1% мин. {low}"; the lows: "мин."); the decimal point kept as in the figures the code formats |
| frame / slowest frame / simulation tick | кадр / самый медленный кадр, самые медленные кадры / такт симуляции |
| units of measure | мс, нс, МБ, ГБ, ГБ/с, Гц; threads: "потоков: {n}" |
| On / Off (setting values) | Вкл. / Выкл.; now {value}: "сейчас: {value}"; "{n} a player": "{n} на игрока" |
| levels named in a sentence | "на высоком уровне"; a list: на уровнях «Низкий», «Средний», «Высокий» и «Ультра» |
| benchmark scenes | Рассвет на море / Дождь на реке / Сумерки (Cinder Basin в сумерках); staged battle: постановочная битва; two Krogoths: два Krogoth |
| numbers | 10 000 (space), 3500 (four digits together) |
| accessibility / reduced motion / reduced flashes | специальные возможности (caption СПЕЦВОЗМОЖНОСТИ) / меньше движения / меньше вспышек |
| high-contrast cursor / message duration | контрастный курсор / время сообщений |
| keyboard pointer / screen reader | курсор клавишами / экранный диктор |
| battle event history / controller dead zone | журнал битвы / мёртвая зона стика |
| unit skin / skin pack / base skin | облик юнитов / пакет обликов / базовый облик |
| Christmas (skin) / paint texture | Рождество / текстура окраски |
| achievement / achievements | достижение / достижения |
| commander dossier / service record | досье командира / послужной список |
| rank and feats | ранг и подвиги |
| map source / deactivation | источник карты / отключение |
| reserve / transport berth (Conquest) | резерв / транспортное место |
| compatibility playback | совместимый просмотр |
| livery / pattern / texture slot / trim (skin editor) | окраска / узор / слот текстуры / окантовка |

## Achievements (the redone list)

| English | Russian |
|---|---|
| tiers (the badge's metal, nouns) | БРОНЗА / СЕРЕБРО / ЗОЛОТО / ПЛАТИНА ("{tier}: {done} из {total}") |
| categories | КАМПАНИЯ / СХВАТКА / СЕТЕВАЯ ИГРА / БОЕВЫЕ (the shared COMBAT filter) / ИНДУСТРИЯ / СЛУЖБА |
| service record / FROM YOUR SERVICE RECORD | послужной список / ИЗ ПОСЛУЖНОГО СПИСКА; earned from it: получено по послужному списку |
| secret achievement / SECRET (tag) | секретное достижение / СЕКРЕТ |
| earned / ACHIEVEMENT UNLOCKED | получено ("Получено {date}", ДОСТИЖЕНИЙ ПОЛУЧЕНО; filters ПОЛУЧЕНО / В ПРОЦЕССЕ / НЕ ПОЛУЧЕНО) / ДОСТИЖЕНИЕ ОТКРЫТО (as "Достижение открыто: {achievement}") |
| THE MOMENT (the picture taken when earned) | ТОТ САМЫЙ МОМЕНТ; "{clock} с начала битвы"; the picture: снимок |
| descriptions | imperative with вы: missions "Пройдите…", battles "Выиграйте…"; in one battle: за одну битву (a count), в одной битве (a peak: income, army); on Hard: на сложном уровне; alone: без союзников; different: разных; metal "Добудьте", energy "Выработайте" (as ДОБЫТО МЕТАЛЛА / ВЫРАБОТАНО ЭНЕРГИИ); kinds of unit: "юниты 25 разных типов" |
| numbers in achievements | 1 000, 10 000 (four digits grouped too, as the list's first translations did, so the series reads alike) |
| terms in them | шахта Moho, термоядерный реактор, ракетная шахта, стратегическая ракета, дальнобойная / скорострельная плазменная пушка, экспериментальный юнит, солнечный коллектор, перевезти на транспорте, переработать обломки, урок обучения, битва в «Завоевании» |

## Career ranks

A title every five levels, in this order (ранг, as "Высший ранг"); real Russian ranks, Капрал and
Бригадир the historical ones (the insignia are US-style: Brigadier has one general's star).

| English | Russian |
|---|---|
| Recruit / Private / Private First Class / Lance Corporal / Corporal | Новобранец / Рядовой / Ефрейтор / Младший сержант / Капрал |
| Sergeant / Staff Sergeant / Master Sergeant / Sergeant Major / Warrant Officer | Сержант / Старший сержант / Старшина / Прапорщик / Старший прапорщик |
| Second Lieutenant / Lieutenant / Captain / Major / Lieutenant Colonel / Colonel | Младший лейтенант / Лейтенант / Капитан / Майор / Подполковник / Полковник |
| Brigadier / Major General / Lieutenant General / General / Commander | Бригадир / Генерал-майор / Генерал-лейтенант / Генерал / Командир |
| next rank / level | "Следующий ранг: {rank} на уровне {level}" / уровень |
| service star / SERVICE STARS (past the top rank) | звезда выслуги / ЗВЁЗДЫ ВЫСЛУГИ ("{xp} / {next} XP до следующей звезды") |
| PROMOTED / Promoted: {rank} | ПОВЫШЕНИЕ / Повышение: {rank} |
| SERVICE RECORD (what a battle did for the career) | ПОСЛУЖНОЙ СПИСОК |
| XP sources (chips "{source} +{xp}") | Уничтожение, Строительство, Захваты, Экономика, Время в игре, Итог, Битва, Достижения |

## Profile page

| English | Russian |
|---|---|
| profile / PROFILE (the launcher's page) | профиль / ПРОФИЛЬ |
| commanders (the players' profiles on a device) | командиры, lower case (КОМАНДИРЫ НА ЭТОМ УСТРОЙСТВЕ, as ИГРОКИ НА ЭТОМ УСТРОЙСТВЕ); "their" of one: этого командира, с его настройками |
| ACTIVE PROFILE (tag) | АКТИВНЫЙ ПРОФИЛЬ |
| SWITCH TO (a profile) | ВЫБРАТЬ (as "Выберите, кто играет"; ПЕРЕКЛЮЧИТЬСЯ measures 1.7 times the English); the row's actions in a sentence: выбрать, изменить или удалить |
| MANAGE PROFILES | УПРАВЛЕНИЕ ПРОФИЛЯМИ (the menu's full-width 320 px button, about 180 px of text) |
| OPEN PROFILE FOLDER | ОТКРЫТЬ ПАПКУ ПРОФИЛЯ (two header buttons only; the skins page's four shortened theirs to ПАПКА ОБЛИКОВ) |
| a profile's pictures (its files) | изображения (as the Pictures folder, «Изображения») |
| showcase (achievements shown on the profile) | витрина (ВИТРИНА); SHOW ON / REMOVE FROM YOUR PROFILE: ПОКАЗАТЬ В ПРОФИЛЕ / УБРАТЬ ИЗ ПРОФИЛЯ (quoted in a sentence: нажмите «ПОКАЗАТЬ В ПРОФИЛЕ») |
| motto | девиз (ДЕВИЗ) |
| allegiance | сторона ("Сторона: {faction}") |
| career XP | опыт карьеры ("Опыт карьеры: {xp}", ОПЫТ КАРЬЕРЫ, XP BY SOURCE: ИСТОЧНИКИ ОПЫТА); "XP" kept only beside figures ("{xp} / {next} XP") |
| Serving since {date} / {time} played | На службе с {date} / В игре: {time} |
| overview tiles and cards | БИТВ ЗАВЕРШЕНО, "{percent}% побед", ПО РЕЖИМАМ (columns Режим / Победы / Завершено / Время, as the tiles), ЛЮБИМЫЕ ЮНИТЫ, СТОРОНЫ, ПОСЛЕДНИЕ БИТВЫ |
| BEST BATTLES (each figure the best of one battle) | РЕКОРДЫ БИТВ ("Ваши рекорды появятся здесь…"); rows Уничтожено врагов, Построено юнитов и зданий, Уничтожено вашим Командиром, Доход металла / энергии, Крупнейшая армия (металл) |

## Map editor

| English | Russian |
|---|---|
| tools (full names / the 51 and 65 px tiles) | ПОДНЯТЬ, ОПУСТИТЬ, СГЛАДИТЬ, ВЫРОВНЯТЬ, ЗАДАТЬ ВЫСОТУ (tile УРОВЕНЬ); РИСОВАТЬ ТАЙЛЫ (ТАЙЛЫ), КОПИЯ И ВСТАВКА (КОПИЯ), ОБЪЕКТЫ, СТАРТОВЫЕ ПОЗИЦИИ (СТАРТЫ, also the view's switch); a list of them: "Поднять, опустить, сгладить, выровнять, уровень" |
| start position | стартовая позиция; the tool in a sentence: инструмент «Стартовые позиции» |
| brush / eraser / stroke | кисть / ластик / мазок ("[ ]: размер кисти", "размер ластика") |
| the level (Level tool's height) | уровень: "довести землю до уровня", "взять уровень" (Ctrl+click takes it) |
| let go of (a copy, a selection) | сбросить (сбросить копию, as FORGET COPY: СБРОСИТЬ КОПИЮ) |
| status bar's buttons ("Left: …") | ЛКМ, as ПКМ / СКМ ("ЛКМ: поднять · Shift: опустить"); Ctrl+щелчок, Ctrl+протяжка, Протяжка |
| status fragments (lower case) | высота {h} · глубина {d} ("{d} under water": the water's depth) · тайл {x}, {z}; Наведите курсор на карту |
| KEYS & MOUSE (118 px button) / KEYS AND MOUSE (the page) | УПРАВЛЕНИЕ (КЛАВИШИ И МЫШЬ measures 137 px) / КЛАВИШИ И МЫШЬ; sections ВИД / ИНСТРУМЕНТЫ / КАРТА / ПРАВКА (EDITS) |
| FRAME MAP (96 px) / the whole map in view | ВСЯ КАРТА / вся карта в кадре |
| view switches' terms | сетка тайлов, изолиния (contour), непроходимая земля (impassable ground), маркеры стартовых позиций |
| PROBLEMS: {n} / WHAT TO PUT RIGHT / Show me | ПРОБЛЕМЫ: {n} (as the launcher's ПРОБЛЕМЫ ({n})) / ЧТО ИСПРАВИТЬ / Показать на карте; warnings: предупреждения |
| TEST PLAY / BACK TO THE EDITOR | ИСПЫТАТЬ / В РЕДАКТОР (as В ГЛАВНОЕ МЕНЮ; the result screen's button is 150 px) |
| SAVE AS (the dialog) / SAVE AS… (button) | СОХРАНИТЬ КАК / СОХР. КАК… |
| a map / mod of its own | отдельная карта / отдельный мод |
| a map's name where it clashes | имя ("выберите другое имя", "под другим именем"); the field: НАЗВАНИЕ |
| REPLACE YOUR MAP? / REPLACE IT | ЗАМЕНИТЬ ВАШУ КАРТУ? / ЗАМЕНИТЬ |
| amphibious units | юниты-амфибии |

## Map editor: view switches and key caps

| English | ru |
|---|---|
| GRID (switch: the tiles' grid) | СЕТКА |
| HEIGHTS (switch: the height map) | ВЫСОТЫ |
| BLOCKED (switch: impassable ground) | ПРЕГРАДЫ |
| Arrows (key cap: the arrow keys) | Стрелки |
| Wheel (key cap: the mouse wheel) | Колесо |

## Map editor: pictures, features and the copy tool

| English | ru |
|---|---|
| picture (a file imported or exported) / Pictures (file filter) | изображение / Изображения |
| grey picture | изображение в оттенках серого |
| import / export | импорт / экспорт (IMPORT, IMPORT… → ИМПОРТ, ИМПОРТ… as EXPORT… ЭКСПОРТ…; IMPORT A PICTURE → ИМПОРТ ИЗОБРАЖЕНИЯ; undo names Импорт высот / Импорт изображения) |
| the map's art (its tiles as painted) | рисунок (as ART: РИСУНОК; THE WHOLE MAP'S ART → РИСУНОК ВСЕЙ КАРТЫ) |
| tile brush / brush size | кисть тайлов (A TILE BRUSH → КИСТЬ ТАЙЛОВ) / РАЗМЕР КИСТИ |
| the ground's heights | высоты земли |
| USE IT AS (over the three choices) | ИСПОЛЬЗОВАТЬ КАК (the 620 px dialog has room) |
| LOWEST / HIGHEST (heights black and white become) | МИНИМУМ / МАКСИМУМ |
| "{n} tiles" (the brush slider's value box, sized to "64 tiles") | "{n} тайл." (as пикс.; «Тайлов: {n}» is far too wide for the box) |
| map cells | клетки ("Клетки: {w} × {h} · {world}", "Непроходимых клеток: {n}", "пиксель на клетку") |
| dither | дизеринг |
| drop (a file) on the window | перетащить в окно; key cap Drop a picture → Перетащить изображение |
| (Ctrl+Z takes it back) | (Ctrl+Z — отменить) |
| Alt+click (take the feature under the pointer) | Alt+щелчок; "Щёлкните с Alt по объекту, чтобы взять его" |
| PLACE / SCATTER (placing modes) / RANDOM | ПОШТУЧНО / ВРАЗБРОС / СЛУЧАЙНО (three adverbs; the segments share ~180 px beside the chip) |
| DENSITY | ПЛОТНОСТЬ |
| ALL WORLDS / "{n} of every world" | ВСЕ МИРЫ / "Для всех миров: {n}" (as "Для {planet}: {n}") |
| PASTE WITH / RELATIVE (a relative paste) | ВСТАВЛЯТЬ ТАКЖЕ / ОТНОСИТЕЛЬНО (ВЫСОТЫ, ОТНОСИТЕЛЬНО, ОБЪЕКТЫ fit on one row of the 272 px panel) |
| a copy (the copy tool's) | копия (подъёмы и впадины копии) |

## Mod validator (the launcher's data check and its report)

| English | ru |
|---|---|
| data check / the check | проверка данных / проверка (group THE CHECK: ПРОВЕРКА) |
| findings (all of a report's, any severity) | проблемы, as the map editor's ПРОБЛЕМЫ and VS Code's panel ("Поиск проблем…", "Скрыто проблем…: {n}"); "No findings match": Ничего не найдено по фильтрам и поиску (never "Нет проблем…", read as "all is well") |
| severities: ERROR / WARNING / NOTE (filters ERRORS / WARNINGS / NOTES) | ОШИБКА / ПРЕДУПРЕЖДЕНИЕ / ЗАМЕЧАНИЕ (ОШИБКИ / ПРЕДУПРЕЖДЕНИЯ / ЗАМЕЧАНИЯ), as GCC's Russian "замечание" for note |
| INHERITED / CAUSED BY THIS MOD (origins) | УНАСЛЕДОВАНО / ВЫЗВАНО ЭТИМ МОДОМ: short neuter participles, so the one key reads as both the filter chip (as INSTALLED: УСТАНОВЛЕНО) and a single finding's tag; in a sentence: фильтр «УНАСЛЕДОВАНО» |
| count against the mod / not its doing | ставить в вину моду ("в вину моду они не ставятся") / мод тут ни при чём |
| dependency (a mod another needs) / the original game | зависимость / оригинальная игра (not исходная) |
| badges ERRORS FOUND / NOTHING TO CHECK | НАЙДЕНЫ ОШИБКИ / НЕЧЕГО ПРОВЕРЯТЬ (beside НАЙДЕНЫ ПРОБЛЕМЫ, СБОЙ ПРОВЕРКИ, ДАННЫЕ ПРОВЕРЕНЫ) |
| groups | УПАКОВКА (PACKAGING), ФАЙЛЫ, СКРИПТЫ, МОДЕЛИ И ТЕКСТУРЫ, АНИМАЦИИ И ЭФФЕКТЫ, ЗВУКИ, МЕНЮ И СТОРОНЫ (build menus and factions); "&" is И |
| HOW TO FIX / HIDE THE FIX | КАК ИСПРАВИТЬ / СКРЫТЬ РЕШЕНИЕ (the fix: решение) |
| where a finding is: line {n} / in {archive} | строка {n} / в {archive} |
| progress counts | "Файлов: {checked} из {total}", "Файлов оригинальной игры: {checked} из {total}" (as "Карт: {n} из {total}") |
| MOD INFO on the Mods page (in a sentence) | кнопка «О МОДЕ» на странице «Моды» |
| OVERVIEW / FINDINGS (the report's tabs) | ОБЗОР / ПРОБЛЕМЫ (in a sentence: на вкладке «ПРОБЛЕМЫ»); READ THE REVIEW: ЧИТАТЬ ОБЗОР ПРОЕКТА (ОБЗОР alone is the OVERVIEW tab, a key shared with the profile page) |
| the verdicts (OVERVIEW's heading) | Нужно исправить ошибки / Стоит изучить предупреждения / Проблем не найдено / Для проверок данных ничего нет / Проверка не завершена |
| IN THIS MOD / FIX FIRST / SEE ALL {n} FINDINGS | В ЭТОМ МОДЕ / СНАЧАЛА ИСПРАВИТЬ / ВСЕ ПРОБЛЕМЫ ({n}) |
| {n} OF ONE KIND (a card of findings with one fix) / SHOW FEWER | ОДНОТИПНЫХ: {n} / СВЕРНУТЬ (beside ПОКАЗАТЬ ВСЕ {n}) |
| HOW TA REBORN PLAYS IT (what a mod uses of TA's later engines) | КАК ЭТО РАБОТАЕТ В TA REBORN; tags PLAYED / NOT USED: РАБОТАЕТ / НЕ ИСПОЛЬЗУЕТСЯ (the "how" line of plural things: "Не используются: …"); as made {played}: как задумано — {played}; plays it itself: воспроизводит сам; its own way: по-своему |
| TA's later (patched) engines / a later patch / TA's own engine | более поздние движки TA, движки с патчами последующих лет / более поздний патч / оригинальный движок TA |
| build list (TA 3.1's [CANBUILD], the computer players') | список постройки (as меню постройки, вариант постройки); its owner: владелец списка постройки, lower case as the other reference labels |
| menu pages / download lists (download/*.tdf) | страницы меню / списки загружаемых юнитов (TA's downloadable units) |
| build place (a 64-pixel place of the side panel's grid) / page arrows / orders | ячейка (меню постройки) / стрелки страниц / приказы; side panel: боковая панель; pixels after a figure: пикс. |
| modern interface | современный интерфейс (beside классический интерфейс) |
| Seen in play (the project review's) / known gaps: {n} | Проверено в игре / известных проблем: {n} |
| RUN AGAIN in a sentence | нажмите «НАЧАТЬ СНОВА» (the key is the benchmark's too) |
| COMPATIBILITY (the mod library's caption) | СОВМЕСТИМОСТЬ |
| weapon ID / unit value | ID оружия / значение юнита; a count of weapons: "Видов оружия: {n}" |

## Conquest (the galaxy campaign)

| English | ru |
|---|---|
| system / capital / home (system) | система / столица / дом (tag ДОМ, "{world} · ВАШ ДОМ"; родная система in a sentence) |
| a system taken / holding out ({name} TAKEN / HOLDS) | СИСТЕМА {name} ЗАХВАЧЕНА / СИСТЕМА {name} УСТОЯЛА (the noun carries the gender; "Система устояла." in a sentence) |
| BATTLE FOR {name} / BATTLE {n} | БИТВА ЗА {name} / БИТВА {n} (битва; бой only in set phrases: поле боя, в бою, в строю) |
| landing zone / the enemy's bases | зона высадки / базы противника; a veteran that could not land: не смог высадиться |
| veterans / berth / supply | ветераны / место для ветерана, транспортное место / снабжение |
| debrief tiles VETERANS HOME / NEWLY RANKED / FALLEN | ВЕРНУЛИСЬ ДОМОЙ / НОВЫЕ ВЕТЕРАНЫ / ПАВШИЕ (the 150 px tiles; ПАВШИЕ also the section caption) |
| Veterans standing (the live count) | Ветераны в строю |
| jump (between systems) | прыжок ("прыжков от дома до вражеской столицы: {jumps}") |
| CAMPAIGN RECORD (opens the JOURNAL tab) | ЖУРНАЛ КАМПАНИИ ("Журнал этой кампании будет очищен") |
| within reach / out of reach | в пределах досягаемости / вне досягаемости |
| the screen reader's "{name}, {whose} {reach}" | "{name}, {whose}, {reach}": the code trims the trailing comma when {reach} is empty ("Twin Reefs, наша") |
| counts of computer players | "Компьютерных игроков: {n} ({level})" beside "Один компьютерный игрок ({level})" |
| rewards (a system's, names in capitals) | ШАХТЫ / РЕАКТОРЫ / ТАНКОВЫЙ ЗАВОД / АЭРОДРОМ / ВОЕННАЯ АКАДЕМИЯ / СЕНСОРНАЯ СЕТЬ / СКЛАД СНАБЖЕНИЯ; a reward: награда (НАГРАДА, НАГРАДЫ; "Получена / Потеряна награда {reward}: {effect}", so the name stays nominative) |
| IT BRINGS / TAKING IT BRINGS / AT STAKE | ПРИНОСИТ / ЗАХВАТ ПРИНЕСЁТ / НА КОНУ |
| reinforcement (sent by tank plants and airfields) | подкрепление ("Подкрепления из ваших систем: {n}"); "A {unit} joins each battle": "{unit} присоединяется к вам в каждой битве" (the name first, no gender) |
| counter-attack / strike back | контратака (КОНТРАТАКА), контратаковать ("противник часто контратакует"); in sentences удар: it lands = удар обрушится ("До удара осталось битв: {n}"), beat it off = отразить удар (дав здесь / там бой), break it up = сорвать удар |
| UNDER ATTACK / DEFEND / DEFENCE OF {name} | ПОД АТАКОЙ / ОБОРОНЯТЬ (as АТАКОВАТЬ) / ОБОРОНА {name} (as БИТВА ЗА {name}) |
| a defence won / lost ({name} HELD / LOST) | СИСТЕМА {name} УДЕРЖАНА / ПОТЕРЯНА (beside ЗАХВАЧЕНА / УСТОЯЛА); "Битва {n}: система удержана / потеряна / пала под контратакой" |
| YOUR BASE / THE ATTACKERS | ВАША БАЗА / НАПАДАЮЩИЕ; the attacking Commander: атакующий Командир |
| uncharted / chart (systems) | неразведанная (НЕРАЗВЕДАННАЯ СИСТЕМА; legend НЕРАЗВЕДАННЫЕ; predicate "не разведана") / разведывать; BEYOND YOUR SENSORS: ВНЕ ЗОНЫ ВАШИХ СЕНСОРОВ |
| perk (the Commander's, one at each promotion) | перк, the gaming word (ВЫБЕРИТЕ ПЕРК, "Перки Командира", "перк на выбор", "Повышение: перк на выбор"); not способность (a unit's abilities) nor награда (a system's reward) |
| the perks (names in capitals) | БОЕВОЙ ИНЖЕНЕР / УСИЛЕННАЯ БРОНЯ / СНАЙПЕР / ПОЛЕВОЙ ИНСТРУКТОР / КОНДЕНСАТОРЫ / ИНТЕНДАНТ |
| PROMOTION (the choice's title) / CHOOSE / LATER | ПОВЫШЕНИЕ (as PROMOTED) / ВЫБРАТЬ / ПОЗЖЕ; has risen to {rank}: дослужился до ранга {rank} |
| half as fast again / a quarter more / half its energy | в полтора раза быстрее / на четверть больше (урона, снабжения) / вдвое меньше энергии |

## The unit encyclopaedia and its wallpapers (`encyclopaedia.rs`, `showroom.rs`)

| English | Русский |
|---|---|
| STAND / MOVE / FIRE (what the unit on show does: segments of one control) | ПОКОЙ / ДВИЖЕНИЕ / ОГОНЬ |
| YOUR SCREEN (a wallpaper's size, beside 1080p...) | ВАШ ЭКРАН |
| RESET VIEW (the camera back to the front of the unit) | СБРОСИТЬ ВИД |
| BUILT BY / BUILDS (the details' links along the build chain) | КТО СТРОИТ / ЧТО СТРОИТ |
| ALL KINDS (beside ALL SIDES) | ВСЕ ТИПЫ |
| {n} UNITS / {n} OF {total} UNITS (the list's subtitle) | ЮНИТОВ: {n} / ЮНИТОВ: {n} ИЗ {total} |
| Name and numbers (the wallpaper's words, on or off) | Название и ТТХ |

## The battle's result screen (`result.rs`)

| English | Русский |
|---|---|
| SUMMARY / PLAYERS / GRAPHS (the debrief's tabs) | ИТОГИ / ИГРОКИ / ГРАФИКИ |
| BATTLE TIME (a figure card, beside ПОСТРОЕНО, ПОТЕРЯНО, УНИЧТОЖЕНО) | ВРЕМЯ БИТВЫ (as "ВРЕМЯ БИТВЫ {clock}"); a figure that is the best of all: лучший показатель среди всех игроков |
| BEST UNIT / 1 kill, {n} kills | ЛУЧШИЙ ЮНИТ / Уничтожено: 1, Уничтожено: {n} (as "{rank} · уничтожено: {n}") |
| Still standing / Destroyed in the battle (the best unit's fate) | Всё ещё в строю / Пал в бою (not «Уничтожен», which would echo the kills line above it) |
| KEY MOMENTS / First blood | КЛЮЧЕВЫЕ МОМЕНТЫ / Первая кровь (as FIRST BLOOD: ПЕРВАЯ КРОВЬ) |
| the moments' lines | chronicle present, gender-free as "{player} передаёт вам юнит": "Вы теряете Командира", "{player} запускает ядерную ракету"; never "Игрок {player}" ({player} may be «Игрок 3») |
| The largest battle | Крупнейшее сражение (as THE BIGGEST BATTLE) — уничтожено юнитов: {n} |
| what decided the battle | Вражеский Командир пал / Все вражеские Командиры пали / Враг разбит / Ваш Командир пал на {clock} / Ваши силы разбиты |
| REMATCH (the same skirmish again) | РЕВАНШ (as "gg. Реванш?") |

## The launcher's map library (`pages/maps.rs`)

| English | Русский |
|---|---|
| YOUR FAVOURITES (section heading: the starred maps; a count follows each heading) | ВАШЕ ИЗБРАННОЕ |
| TA REBORN MAPS (section heading: TA Reborn's own maps) | КАРТЫ TA REBORN |
| MORE MAPS (section heading: every other map) | ДРУГИЕ КАРТЫ |
| TA REBORN MAP (caption of the banner in a map's details) | КАРТА TA REBORN |
| optimised for its engine / balanced for every start (each start position as good as the others) | оптимизированы под его движок / все стартовые позиции равноценны |

## Seasonal decorations and field notes (`seasonal.rs`, the skins page, settings)

| English | Русский |
|---|---|
| Halloween (the skin; the SEASONAL THEME choice) | Хэллоуин / ХЭЛЛОУИН (beside Рождество, РОЖДЕСТВО, НОВЫЙ ГОД) |
| FESTIVE / NEW YEAR / SPOOKY FIELD NOTES (the notes plate's heading in season) | ПРАЗДНИЧНЫЕ / НОВОГОДНИЕ / ЖУТКИЕ ПОЛЕВЫЕ ЗАМЕТКИ |
| witching purple / pumpkin orange (the Halloween skin) | колдовской фиолетовый / тыквенно-оранжевый (окантовка) |
| the season's funny notes | jokes retold, not translated: Дед Мороз (Santa), ёлка, гирлянды, колядки, носки (stockings), салют (fireworks: "ядерная шахта салютует громче"), куранты (the countdown), проводы старого года (out with the old), уходящий год, «Празднуйте ответственно» |
| Silent night, holy night / Trick or treat? | «Тихая ночь, дивная ночь» / «Сладость или гадость?» |
| jump scare / never split the party | скример / никогда не разделяйтесь |
| radar blips / sneak attack | отметки (на радаре) / внезапная атака |
| fog of war / True (line of sight) in a sentence | туман войны / обзор «Реальный» |

## Skins: Reborn and Heritage (`pages/skins.rs`, `pages/skin_editor.rs`, `skins.rs`)

After the caption ЮНИТЫ the name stands as a genitive ("юниты TA Reborn"), so the choice is just TA REBORN. TA is feminine, as "самой TA": "TA, какой она была", "какими их нарисовала TA".

| English | Russian |
|---|---|
| TANKS (the skin preview's units: a tab beside KBOTS, COMMANDERS, AIRCRAFT, BUILDINGS) | ТАНКИ |
| TA REBORN'S / TA'S ORIGINALS (the skin editor's UNITS row: what a skin is painted on) | TA REBORN / ОРИГИНАЛЫ TA |
| TA's originals (the LIVERY fact of a skin painted on TA's own units) | Оригиналы TA |
| FADE (slider beside WEAR and GRIME: paint bleached by the sun) | ВЫЦВЕТАНИЕ (the tip: краска, выгоревшая на солнце) |
| Rebuilt in detail (Reborn's card) / TA as it was (Heritage's card) | Детальная реконструкция / TA, какой она была |
| armour plates / bolts / road wheels (what Reborn adds to every unit) | бронеплиты / болты / опорные катки |

## Enemy Commanders' transmissions (`chatter.rs`, `hud/comms.rs`)

| English | Русский |
|---|---|
| INCOMING TRANSMISSION | ВХОДЯЩАЯ ПЕРЕДАЧА |
| persona titles FIELD COMMANDER / MACHINE INTELLIGENCE / MERCENARY | БОЕВОЙ КОМАНДИР (not «полевой командир», which reads as an insurgent warlord) / МАШИННЫЙ РАЗУМ / НАЁМНИК |
| the voices | all three say вы; the officer and the mercenary speak of themselves in the masculine ("я изучил", "я тренировался"), the officer's side is мы; the machine talks in log entries ("Дерзость зафиксирована.", "Игнорирую.", "Исход: повторяется.") |
| patterning / designation / rematch | оцифровка ("Приготовьтесь к оцифровке") / обозначение / реванш |
| {unit} (a unit's name, its gender unknown) | no word agrees with it: present or future verbs ("{unit} выбывает из боя", "{unit} нас больше не побеспокоит"), labels ("Удалено с поля боя: {unit}", "Потеря: {unit}"), {unit} after a noun or preposition ("Потеря {unit}", "За {unit} вы заплатите", "только не {unit}"), an anchoring noun ("Ваш юнит {unit} демонтирован", "Цель {unit} ликвидирована", "хорошая машина: {unit}"); "my {unit}" as "{unit}!" or "это моё" |
| the officer's idioms | К бою (Begin), Не числом, а умением (quality over quantity), Для вас война окончена |

## A replay made into a video (`film.rs`)

| English | Русский |
|---|---|
| video / EXPORT VIDEO / Export video (the dialog) / EXPORTING VIDEO | видео / ЭКСПОРТ ВИДЕО (116 px in the replay bar's fixed 126 px button) / Экспорт видео / ЭКСПОРТ ВИДЕО; Video export cancelled: Экспорт видео отменён |
| film (what goes in the video) | снимать (WHAT: ЧТО СНИМАТЬ; "Здесь нечего снимать") |
| picture (one frame of the video) / PICTURE (the row: size and frame rate) / ON THE PICTURE (the overlays) | кадр ("Кадр {frame} из {total}") / ИЗОБРАЖЕНИЕ (paired with ЗВУК) / В КАДРЕ |
| pictures a second (the video's frame rate) | кадр/с ("30 кадр/с", "{rate} кадр/с"), as video editors write it; the benchmark keeps FPS |
| big moments / biggest (the replay scan's) | главные моменты (as "Поиск главных моментов повтора…"; highlights stay лучшие моменты); "Главных моментов: {count}"; the biggest fight: крупнейший бой |
| WHAT choices | Вся битва / Фрагмент (a stretch) / Главные моменты |
| FROM HERE / TO HERE (set the stretch's ends) | НАЧАЛО ЗДЕСЬ / КОНЕЦ ЗДЕСЬ (beside "С {from} до {to}") |
| QUALITY (neuter, качество) | Стандартное / Высокое / Лучшее |
| CAMERA choices / named in a line | Режиссёр / Кино / Как настроено; Режиссёрская камера / Кинематографичная камера / Ваша камера |
| scoreboard / kill feed / titles | табло / лента уничтожений / титры |
| Battle sounds | Звуки битвы |
| Videos folder (Windows' own) | «Видео» ("сохраняется в папку «Видео»", as «Изображения») |
| length / size (of the video) | длительность / размер ("Длительность {length} · {size}"); ГБ / МБ |
| going to a time in the replay | перемотка ("Перемотка повтора к {time}…") |
| SHOW IN FOLDER / WATCH IT / BACK TO THE REPLAY (170 px buttons) | ПОКАЗАТЬ В ПАПКЕ (139 px) / СМОТРЕТЬ / К ПОВТОРУ (as К БРИФИНГУ) |
| VIDEO SAVED / THE VIDEO COULD NOT BE MADE | ВИДЕО СОХРАНЕНО / НЕ УДАЛОСЬ СОЗДАТЬ ВИДЕО |
| VS (the title card) | VS, as Russian esports writes it |

## The companion screen (`companion.rs`, `hud/companion.rs`)

| English | Русский |
|---|---|
| companion screen (the second window: button, setting, window title) | экран-компаньон (as приложение-компаньон); ЭКРАН-КОМПАНЬОН / ЗАКРЫТЬ ЭКРАН-КОМПАНЬОН measure as the English in the 300 px pause menu (ВСПОМОГАТЕЛЬНЫЙ ЭКРАН would not fit; второй экран clashes with the display) |
| second display | второй монитор (экран is the companion's own word) |
| intel / beside the battle | разведданные / рядом с полем боя |
| Click: look · Right-click: send the selection · Alt+click: ping | Щелчок: показать · Щелчок ПКМ: отправить выбранные · Alt+щелчок: отметить точку |
| LOOK (an event's 44 px tag) | ПОКАЗ. (ПОКАЗАТЬ measures 52 px; as SAVE → СОХР.) |
| {count} kills (a player's card, 220 px at the narrowest) | Уничт.: {count} (as УНИЧТ. {n}); units: Юнитов: {count} |
| Idle constructors: {count} (beside ДАЛЕЕ) | Строители без дела: {count} (as the factories' БЕЗ ДЕЛА tag above it) |
| EVENTS / Nothing yet. / {count} more | СОБЫТИЯ / Пока ничего. / ещё {count} |

## The benchmark's eight scenes and two tests (`benchmark.rs`, `pages/benchmark.rs`, `ta_ui/src/benchmark.rs`)

| English | Русский |
|---|---|
| quick run / full run (QUICK RUN, FULL RUN) | быстрый прогон / полный прогон (БЫСТРЫЙ ПРОГОН measures 129 px in the 180/220 px button; «тест» stays for the two tests); QUICK BENCHMARK: БЫСТРЫЙ БЕНЧМАРК; first benchmark: первый бенчмарк |
| effects test | тест эффектов (ТЕСТ ЭФФЕКТОВ; "Тест эффектов · Высокий", as "Тест настроек · {level}") |
| "{effect} off" / everything on | "{effect}: выкл." (a setting's state, so no gender to agree: "Высокий, {effect}: выкл.") / "Высокий, всё включено" |
| what an effect costs | цена (ЦЕНА КАЖДОГО ЭФФЕКТА, "чтобы узнать цену каждого"); the copied line "{effect}: {ms} мс ({share}%) на высоком уровне" (стоит / стоят would have to agree with {effect}) |
| scene (of the benchmark) / THE SCENES / EACH SCENE / ALL SCENES | сцена / СЦЕНЫ / ПО СЦЕНАМ (as ПО БИТВАМ) / ВСЕ СЦЕНЫ (as ВСЕ ТРИ) |
| the storm (Highland Rift's scene) / the twenty thousand | гроза ("секунды грозы") / сцена «Двадцать тысяч» |
| scene moments | Рассвет на море / В кабине Stumpy / Гроза / В пустоте / Растущая база / Двадцать тысяч / Сумерки / Полночь |
| world kinds (chips) | Мир лавы (as the maps' МИР ЛАВЫ), Мир металла, Космический мир (not «Мир в космосе», read as "peace in space"); in sentences adjectives: на металлическом мире, на лавовом мире |
| the other chips | Отражения в воде, Боевые флоты, Война в небе (Air war) / Воздушные бои (Dogfights), Вид из кабины, 3D-лес, Переправа, Дождь и молнии, Горящий лес, Штурм башен, Эстакады (Gantries: bridges across the void), Нанолаты (as нанолат in the timeline; "под лучами нанолатов"), Закат в пустыне, 20 000 юнитов, Стратегический зум (Supreme Commander's Russian term), Массовая битва, Ядерный удар, Салют, Снегопад, Ночные огни |
| effects (labels, as the settings' names) | Трассировка лучей, Отражения в воде, Мягкие тени, Фоновое затенение, 3D-деревья и камни, HD-поверхности, Погода, Марево, Полное разрешение |
| renderer's passes (legend) | проходы рендеринга: Тени, Трассировка лучей, Отражения, Сцена (The scene), Частицы, Вода и эффекты, Свечение (Bloom, as the setting), Сборка кадра (Finishing: the composite pass), Между проходами |
| stutters (frames of 50 ms+, 3× the usual) | фризы (ФРИЗЫ, "Фризов: {n}", "фризы: кадры от 50 мс, в три и более раз дольше обычного") |
| MIDDLE FRAME / 1 IN 10 (100) LONGER THAN | МЕДИАННЫЙ КАДР (средний is the mean, as "среднее время" in the table's legend) / 1 ИЗ 10 (100) ДОЛЬШЕ (the figure under it completes "дольше 16.7 мс") |
| FRAME TIMES / histogram buckets | ВРЕМЯ КАДРОВ / "{fps}+", "менее {fps}"; caption "Частота кадров, кадр за кадром" |
| WHERE THE GRAPHICS CARD'S TIME GOES | НА ЧТО УХОДИТ ВРЕМЯ ВИДЕОКАРТЫ |
| units at most / a frame | "юнитов: до {units}" / "на кадр" ("видеокарта {gpu} мс на кадр") |

## Photo mode, the jukebox and the pause menu (`photo.rs`, `jukebox.rs`, `app.rs`, `battle_history.rs`)

Widths measured in Bahnschrift SemiBold, as the panel draws them.

| English | Русский |
|---|---|
| THE BATTLE (caption: the pause menu's group of buttons; photo mode's pace) | БИТВА |
| FROZEN / SLOW / LIVE (photo mode's pace, under БИТВА) | ЗАМЕРЛА / ЗАМЕДЛЕНА / ИДЁТ, agreeing with БИТВА as the subtitles БИТВА ЗАМЕРЛА / БИТВА ЗАМЕДЛЕНА (THE BATTLE IN SLOW MOTION) / БИТВА ИДЁТ; the row is 320 px (they measure 267 with padding) |
| the looks (tiles ~102 px wide, 10.5 px type) | ОРИГИНАЛ (NATURAL: the battle's own picture; ЕСТЕСТВЕННЫЙ is 1.7× the English) / КИНО (as Cinematic) / ВОЕННОЕ КИНО (81 px) / НУАР / ВИНТАЖ / АРКТИКА / ЗОЛОТОЙ ЧАС / ТЕПЛОВИЗОР / НОЧНОЕ ЗРЕНИЕ (NIGHT SIGHT, 87 px; НОЧНОЕ ВИДЕНИЕ measures 97 px, too wide) |
| a look / RESET THE LOOK | стиль / СБРОСИТЬ СТИЛЬ (130 px in the 150 px button; СБРОСИТЬ ВИД is the encyclopaedia's camera) |
| THE WORLD'S / RAIN (weather choices) | КАК В МИРЕ (as THE BATTLE'S OWN: КАК В БИТВЕ) / ДОЖДЬ |
| long / wide lenses | длиннофокусные / широкоугольные объективы |
| TILT (the horizon's) / CONTRAST / WARMTH / FILM GRAIN / COLOUR FRINGE | НАКЛОН / КОНТРАСТ / ТЕПЛО (as Instagram's) / ЗЕРНО ПЛЁНКИ / ЦВЕТНАЯ КАЙМА (chromatic aberration, as «фиолетовая кайма»; the hint: расхождение цветов) |
| SHAPE (the aspect ratio) / GUIDES / THIRDS / GOLDEN / CENTRE / CAPTION | ФОРМАТ / СЕТКА (feminine: "Видна только на экране") / ТРЕТИ / ЗОЛОТОЕ (сечение) / ЦЕНТР / ПОДПИСЬ |
| track (jukebox) | трек ("ТРЕК {n} ИЗ {count}", "1 ТРЕК", "ТРЕКОВ: {n}") |
| NOW PLAYING / Silent (nothing plays) | СЕЙЧАС ИГРАЕТ / Тишина |
| Shuffle / Repeat (the jukebox's modes) | Перемешивание / Повтор, nouns ("Перемешивание: выкл."); the skin editor's SHUFFLE button stays ПЕРЕМЕШАТЬ |
| notice (the battle event history's) / battle session | сообщение, as its filter ("1 СООБЩЕНИЕ", "СООБЩЕНИЙ: {n}") / сеанс битвы |
| SCREENSHOT (key hint) / YOUR FORCES | СНИМОК ЭКРАНА / ВАШИ СИЛЫ |

Shared keys that read wrongly in photo mode (one key serves both places; only the code can part them): LOOK (its tab) is the companion's ПОКАЗ., CLEAR (no weather) is a button's СБРОСИТЬ, LEVEL (sets the horizon level, a 64 px button) is the editor's УРОВЕНЬ (68 px).

## The Galactic War page: chronicle and archive (`lore.rs`, `timeline.rs`, `timeline/galaxy.rs`)

The chronicle told plainly and a little solemn; each archive document in its own voice: CORE's edict and directive in cold officialese ("подлежит", "надлежит", "прекращает существование"), the intercept in capitals with orders as infinitives, the ARM manual wry, to "вы".

| English | Русский |
|---|---|
| the chronicle (PLAY THE CHRONICLE, THE CHRONICLE: {chapter}) / chapter | хроника (ЧИТАТЬ ХРОНИКУ, ХРОНИКА: {chapter}) / глава |
| FIGHT IT (caption before the campaigns that fight a chapter) / the CAMPAIGNS tab | В БОЙ / вкладка «КАМПАНИИ» |
| eras THE PATTERNING / THE ARCHIVE | ОЦИФРОВКА / АРХИВ (beside ЭПОХА CORE, ГАЛАКТИЧЕСКАЯ ВОЙНА, TOTAL ANNIHILATION) |
| patterning / patterned / a pattern (a mind's copy) | оцифровка / оцифрованный / слепок (сознания): "слепки сознания лучших воинов", "восстанавливаются из слепков", "CORE — это слепок", АРХИВ СЛЕПКОВ |
| the refusers / THE REFUSAL / THE DECLARATION OF THE REFUSAL | отказавшиеся / ОТКАЗ / ДЕКЛАРАЦИЯ ОТКАЗА |
| the (Patterning) Edict / THE EDICT / EDICT ONE | Указ (об оцифровке) / УКАЗ / УКАЗ ПЕРВЫЙ |
| the Council of CORE / collection ships, fleets / patterning hall | Совет CORE / корабли-сборщики, флоты сборщиков / пункт оцифровки |
| the Galactic Gates / a Gate | Галактические Врата / Врата (plural only: "каждые Врата", "Удержите Врата") |
| the Central Consciousness / CORE PRIME, Empyrrean, Hydross, Lusch, Temblor, Gelidus | Центральное Сознание / in Latin, as before (на Empyrrean, к CORE PRIME); THE ARM on the galaxy's key: ARM |
| the ARM's name (the spiral arm and the weapons) | "по спиральному рукаву галактики (arm) ... и по оружию (arms)" |
| the front / THE FRONT (the galaxy's key) / lead from the front | фронт ("на фронт", ПИСЬМО С ФРОНТА) / ЛИНИЯ ФРОНТА / командовать с передовой; field reports: донесения с передовой |
| BURNED WORLDS (the galaxy's key) | ВЫЖЖЕННЫЕ МИРЫ |
| a contingency / DIRECTIVE: CONTINGENCY / the contingency worlds, forces | резервный план на крайний случай / ДИРЕКТИВА: НА КРАЙНИЙ СЛУЧАЙ / резервные миры, силы (the expansion keeps its name, The Core Contingency) |
| vats (the clones') / powered combat suits | чаны ("из чанов — прямо на фронт", ЧАНЫ ДЛЯ КЛОНИРОВАНИЯ) / боевые экзоскелеты (the Commander's "suit" too) |
| walker (the Krogoth) / walking fortress | шагоход / шагающая крепость |
| nanolathe / disintegrator gun / metal maker / power plant | нанолат / дезинтегратор / преобразователь металла / электростанция |
| datelines | ДО ВОЙНЫ, ГОД 3977, ГОДЫ 212–3977, ПОСЛЕ: years without a separator; other numbers spaced (41 006) |
| a document's FROM / TO lines | FROM in the nominative (СОВЕТ CORE); TO in the dative, as an address (ВСЕМ ГРАЖДАНАМ ВСЕХ МИРОВ, ЧАНУ НА EMPYRREAN, ТОМУ, КТО ЭТО ЧИТАЕТ) |
| stamps | ВОССТАНОВЛЕНО, ПО ПОСТАНОВЛЕНИЮ СОВЕТА, ПЕРЕДАНО НА ВСЕХ ЧАСТОТАХ, ДЛЯ СЛУЖЕБНОГО ПОЛЬЗОВАНИЯ (RESTRICTED), ДЛЯ УЧЕБНЫХ ЦЕЛЕЙ, НЕ ДОСТАВЛЕНО, ПЕРЕХВАЧЕНО, ПОДЛЕЖИТ ИСПРАВЛЕНИЮ, ВСКРЫТЬ ПРИ НЕОБХОДИМОСТИ, СЕКРЕТНО |
| FIELD COMMANDERS / TROOPER | БОЕВЫЕ КОМАНДИРЫ (полевой командир reads as a warlord) / боец |
| blacked-out words | [[...]] kept, the words inside translated ([[МИЛЛИАРДЫ]], [[ЛЮБОЙ ЦЕНОЙ]], [[ПОДАЛЬШЕ]]) |
| the CORE machine remembering (its sex unknown) | its "I" takes no gendered past tense ("в родном городе", "Смерть настигала меня", "Я длюсь") |
| faction (in mod reports) | сторона |
| build option | вариант постройки |
| wreckage | обломки |
| feature (map object) | объект |
