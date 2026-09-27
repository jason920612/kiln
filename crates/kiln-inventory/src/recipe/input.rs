//! `CraftingInput`: a crafting grid trimmed to the bounding box of its non-empty slots.

use kiln_item::ItemStack;

#[derive(Debug, Clone, PartialEq)]
pub struct CraftingInput {
    width: usize,
    height: usize,
    items: Vec<ItemStack>,
    ingredient_count: usize,
}

impl CraftingInput {
    /// `CraftingInput.of`.
    pub fn new(width: usize, height: usize, grid: &[ItemStack]) -> Self {
        Self::positioned(width, height, grid).0
    }

    /// `CraftingInput.ofPositioned`: the trimmed input and its left/top offset in the grid.
    pub fn positioned(width: usize, height: usize, grid: &[ItemStack]) -> (Self, usize, usize) {
        let empty = CraftingInput { width: 0, height: 0, items: Vec::new(), ingredient_count: 0 };
        if width == 0 || height == 0 {
            return (empty, 0, 0);
        }
        let (mut left, mut right, mut top, mut bottom) = (width - 1, 0, height - 1, 0);
        let mut any = false;
        for y in 0..height {
            let mut row_empty = true;
            for x in 0..width {
                if !grid[x + y * width].is_empty() {
                    left = left.min(x);
                    right = right.max(x);
                    row_empty = false;
                }
            }
            if !row_empty {
                top = top.min(y);
                bottom = bottom.max(y);
                any = true;
            }
        }
        if !any {
            return (empty, 0, 0);
        }
        let (w, h) = (right - left + 1, bottom - top + 1);
        let items = if w == width && h == height {
            grid.to_vec()
        } else {
            let mut items = Vec::with_capacity(w * h);
            for y in 0..h {
                for x in 0..w {
                    items.push(grid[x + left + (y + top) * width].clone());
                }
            }
            items
        };
        (Self::from_items(w, h, items), left, top)
    }

    /// An input of exactly these items (no trimming).
    pub fn from_items(width: usize, height: usize, items: Vec<ItemStack>) -> Self {
        let ingredient_count = items.iter().filter(|s| !s.is_empty()).count();
        CraftingInput { width, height, items, ingredient_count }
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn size(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.ingredient_count == 0
    }

    pub fn ingredient_count(&self) -> usize {
        self.ingredient_count
    }

    pub fn get(&self, i: usize) -> &ItemStack {
        &self.items[i]
    }

    pub fn get_xy(&self, x: usize, y: usize) -> &ItemStack {
        &self.items[x + y * self.width]
    }

    pub fn items(&self) -> &[ItemStack] {
        &self.items
    }

    /// The non-empty stacks, in grid order.
    pub fn stacks(&self) -> impl Iterator<Item = &ItemStack> {
        self.items.iter().filter(|s| !s.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn trims_to_bounding_box() {
        let s = |n: &str| ItemStack::of(n, 1).unwrap();
        let e = ItemStack::empty();
        let grid = vec![e.clone(), e.clone(), e.clone(), e.clone(), s("stone"), e.clone(), e.clone(), s("dirt"), s("stick")];
        let (input, left, top) = CraftingInput::positioned(3, 3, &grid);
        assert_eq!((input.width(), input.height(), left, top), (2, 2, 1, 1));
        assert_eq!(input.ingredient_count(), 3);
        assert!(input.get_xy(1, 0).is_empty());
        assert_eq!(input.get_xy(0, 1).item_name(), "minecraft:dirt");
        assert!(CraftingInput::new(3, 3, &vec![e; 9]).is_empty());
    }
}
