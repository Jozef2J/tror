/// Container for dockable tabs.
pub mod dock_area;

/// Painting the parts of a dock: tabs, tab bars, separators, the drop overlay.
pub mod painter;

/// Trait for tab-viewing types.
pub mod tab_viewer;

pub use dock_area::{AllowedSplits, DockArea, DraggedTab, DropZone, Labels, TabFocus};
pub use painter::{
    DockPainter, DropButtonPaint, DropPaint, DropRefusal, DropTargetPaint, PartState,
    SectionBandPaint, SectionBoundaryPaint, SeparatorAxis, SeparatorPaint, StylePainter,
    TabBarPaint, TabBarTailPaint, TabBodyPaint, TabClose, TabMeasure, TabPaint, ToolItemPaint,
    ToolMorePaint, ToolSeparatorPaint,
};
pub use tab_viewer::{DockDrop, TabViewer};
