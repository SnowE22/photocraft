# Фаза 4: Content-Aware Inpaint со скринтонами

Дата: 2026-11-05. Ветка: `community/gpu-screentone-preview`.

## Что сделано

- Проверено, что в Photoncraft уже есть полноценный Content-Aware Fill:
  `crates/algo/src/content_aware.rs` + `crates/algo/src/inpaint.rs` реализуют ту же схему, что
  TypeR `inpaint.js`: многоуровневая пирамида, Wexler/PatchMatch EM, поиск патчей по всему окну
  сэмплирования, заполнение от краёв, точные копии исходных пикселей. Команда и диалог на месте:
  `Edit › Content-Aware Fill…` (`edit.contentAwareFill`), окно `caf_ui.rs` с sampling area,
  color/rotation/scale adaptation.
- Новый модуль `crates/algo/src/snow_inpaint.rs` (префикс `snow_`) фиксирует **TypeR-пресет**
  манга-клининга:
  - `STRUCTURE_WEIGHT = 0.6` — документированный вес структурного различия из TypeR;
  - `typer_options()` — `color_adaptation: Adapt::NONE`, без поворотов/масштаба/зеркала, seed 1:
    заливка копирует реальные пиксели, поэтому зерно скринтона и чёрные контуры не размываются
    (в отличие от адаптации цвета, которая подмешивает тон и сглаживает точки);
  - `typer_fill(w, h, ch, img, hole, source)` — вызов существующего `content_aware::fill` с этим
    пресетом.
- Регрессионный тест: скринтон (точки каждые 4 px на светлой бумаге) + чёрная линия, дырка
  «буквы» поперёк линии. Проверяется: пиксели вне дырки не изменены, в заливке есть и тёмные
  точки, и светлая бумага (не плоское пятно), линия продолжается через дырку.

## Тесты

```
cargo test -p photocraft-algo --lib snow_inpaint   # 2 passed
```

Существующие тесты Content-Aware Fill не тронуты (адаптация по умолчанию не менялась, чтобы не
сломать паритет с Photoshop).

## Ручная проверка в GUI

1. Открыть скан со скринтоном и текстом, выделить буквы (Lasso / Magic Wand).
2. `Edit › Content-Aware Fill…` (или Shift+F5 → Fill…), в диалоге выбрать **Color Adaptation:
   None** — это и есть TypeR-поведение: точные копии без размытия.
3. Ожидаемо: текст исчезает, точки скринтона и контурные линии продолжаются без мутного пятна.

## Ограничения

- TypeR-пресет доступен как модуль и как настройка диалога (Color Adaptation: None); отдельной
  кнопки «TypeR manga» в диалоге пока нет (визуальный этап — позже).
- `Shift+F5` в Photoshop — это Edit › Fill…, поэтому шорткат не перевешивался на Content-Aware;
  команда вызывается из меню Edit.
