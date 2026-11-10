---
section: performance
---
- **A canvas's sized texts changing together are damaged once** (zgx): where a canvas's texts have sizes (`CanvasText::extent`, a terminal's runs), what they painted and will paint is damaged as one rectangle, rather than by each label's setters in turn. Before, a label taking another font damaged its text again and measured it to do so. A canvas text without a size damages as before.
