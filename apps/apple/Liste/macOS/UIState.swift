// Window-level UI state shared by the menus and the views: what is
// selected, what is being edited, which view is showing. None of it is
// data; the core owns that.

import Foundation
import ListeKit
import Observation

@MainActor
@Observable
final class UIState {
    enum ViewMode: String {
        case list
        case board
    }

    var selectedTaskId: String?
    var editingTaskId: String?
    var viewMode: ViewMode = ViewMode(rawValue: Preferences.viewMode) ?? .list {
        didSet { Preferences.viewMode = viewMode.rawValue }
    }
    var showInspector = false
    var searchFocusRequest = 0
    var newTaskRequest = 0
    var newListRequest = 0
    var paletteRequest = 0
    var editRequest = 0
    var searchText = ""
    var showPalette = false
    var showFilterBuilder = false
}
