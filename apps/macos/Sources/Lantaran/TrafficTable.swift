import AppKit
import SwiftUI

/// The traffic list. An AppKit table, because SwiftUI's `Table` slows down
/// badly beyond a few thousand rows and a busy phone produces far more.
struct TrafficTable: NSViewRepresentable {
    @Environment(AppModel.self) private var model

    func makeCoordinator() -> Coordinator {
        Coordinator(model: model)
    }

    func makeNSView(context: Context) -> NSScrollView {
        let table = NSTableView()
        table.style = .fullWidth
        table.usesAlternatingRowBackgroundColors = true
        table.allowsMultipleSelection = false
        table.allowsColumnReordering = true
        table.allowsColumnResizing = true
        table.columnAutoresizingStyle = .lastColumnOnlyAutoresizingStyle
        table.rowHeight = 22
        table.intercellSpacing = NSSize(width: 8, height: 0)
        table.autosaveName = "TrafficTable"
        table.autosaveTableColumns = true

        for column in Column.allCases {
            let tableColumn = NSTableColumn(identifier: column.identifier)
            tableColumn.title = column.title
            tableColumn.width = column.width
            tableColumn.minWidth = 20
            table.addTableColumn(tableColumn)
        }

        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        context.coordinator.table = table

        let scroll = NSScrollView()
        scroll.documentView = table
        scroll.hasVerticalScroller = true
        scroll.hasHorizontalScroller = true
        scroll.autohidesScrollers = true
        return scroll
    }

    func updateNSView(_ scroll: NSScrollView, context: Context) {
        // Reading these registers the view for changes to them.
        let revision = model.revision
        let selected = model.selectedFlowID
        context.coordinator.update(revision: revision, selected: selected)
    }

    enum Column: String, CaseIterable {
        case state, id, url, client, method, status, code, time, duration, request, response

        var identifier: NSUserInterfaceItemIdentifier { .init(rawValue) }

        var title: String {
            switch self {
            case .state: ""
            case .id: "ID"
            case .url: "URL"
            case .client: "Client"
            case .method: "Method"
            case .status: "Status"
            case .code: "Code"
            case .time: "Time"
            case .duration: "Duration"
            case .request: "Request"
            case .response: "Response"
            }
        }

        var width: CGFloat {
            switch self {
            case .state: 16
            case .id: 50
            case .url: 320
            case .client: 110
            case .method: 70
            case .status: 80
            case .code: 44
            case .time: 90
            case .duration: 72
            case .request: 70
            case .response: 80
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate {
        let model: AppModel
        weak var table: NSTableView?
        private var shownRevision = -1
        private var settingSelection = false

        private static let timeFormat: DateFormatter = {
            let formatter = DateFormatter()
            formatter.dateFormat = "HH:mm:ss.SSS"
            return formatter
        }()

        init(model: AppModel) {
            self.model = model
        }

        func update(revision: Int, selected: UInt64?) {
            guard let table else { return }
            if revision != shownRevision {
                shownRevision = revision
                table.reloadData()
                if model.followNewest, model.visible.count > 0 {
                    table.scrollRowToVisible(model.visible.count - 1)
                }
            }
            selectRow(for: selected)
        }

        private func selectRow(for id: UInt64?) {
            guard let table else { return }
            let row = id.flatMap { id in model.visible.lastIndex { model.flows[$0].id == id } }
            let wanted = row.map { IndexSet(integer: $0) } ?? IndexSet()
            if table.selectedRowIndexes != wanted {
                settingSelection = true
                table.selectRowIndexes(wanted, byExtendingSelection: false)
                settingSelection = false
            }
        }

        func numberOfRows(in tableView: NSTableView) -> Int {
            model.visible.count
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard !settingSelection, let table else { return }
            let row = table.selectedRow
            model.selectedFlowID = row >= 0 && row < model.visible.count
                ? model.flows[model.visible[row]].id
                : nil
            // A row picked by hand stops the list from jumping away from it.
            if row >= 0, row < model.visible.count - 1 {
                model.followNewest = false
            }
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let tableColumn, let column = Column(rawValue: tableColumn.identifier.rawValue),
                  row < model.visible.count
            else { return nil }
            let flow = model.flows[model.visible[row]]

            if column == .state {
                let view = tableView.makeView(withIdentifier: tableColumn.identifier, owner: nil) as? StateDot
                    ?? StateDot(identifier: tableColumn.identifier)
                view.color = Self.color(for: flow.state)
                return view
            }

            let cell = tableView.makeView(withIdentifier: tableColumn.identifier, owner: nil) as? NSTableCellView
                ?? Self.makeCell(identifier: tableColumn.identifier, column: column)
            cell.textField?.stringValue = Self.text(column, flow)
            cell.textField?.textColor = column == .code && (flow.status ?? 0) >= 400 ? .systemRed : .labelColor
            return cell
        }

        private static func makeCell(identifier: NSUserInterfaceItemIdentifier, column: Column) -> NSTableCellView {
            let cell = NSTableCellView()
            cell.identifier = identifier
            let field = NSTextField(labelWithString: "")
            field.lineBreakMode = column == .url ? .byTruncatingMiddle : .byTruncatingTail
            field.font = .systemFont(ofSize: NSFont.smallSystemFontSize + 1)
            if [.id, .code, .duration, .request, .response, .time].contains(column) {
                field.font = .monospacedDigitSystemFont(ofSize: NSFont.smallSystemFontSize + 1, weight: .regular)
            }
            if [.id, .code, .duration, .request, .response].contains(column) {
                field.alignment = .right
            }
            field.translatesAutoresizingMaskIntoConstraints = false
            cell.addSubview(field)
            cell.textField = field
            NSLayoutConstraint.activate([
                field.leadingAnchor.constraint(equalTo: cell.leadingAnchor, constant: 2),
                field.trailingAnchor.constraint(equalTo: cell.trailingAnchor, constant: -2),
                field.centerYAnchor.constraint(equalTo: cell.centerYAnchor),
            ])
            return cell
        }

        private static func text(_ column: Column, _ flow: Flow) -> String {
            switch column {
            case .state: return ""
            case .id: return String(flow.id)
            case .url: return flow.url
            case .client: return flow.client
            case .method: return flow.method
            case .status:
                switch flow.state {
                case .active: return "Active"
                case .receiving: return "Receiving"
                case .completed: return "Completed"
                case .failed: return "Failed"
                case .tunnel: return "Not decrypted"
                }
            case .code: return flow.status.map(String.init) ?? "-"
            case .time: return timeFormat.string(from: flow.startedAt)
            case .duration: return flow.durationMs.map(duration) ?? "-"
            case .request: return flow.requestSize.map(size) ?? "-"
            case .response: return flow.responseSize.map(size) ?? "-"
            }
        }

        static func duration(_ ms: Double) -> String {
            ms < 1000 ? String(format: "%.0f ms", ms) : String(format: "%.2f s", ms / 1000)
        }

        static func size(_ bytes: UInt64) -> String {
            ByteCountFormatter.string(fromByteCount: Int64(bytes), countStyle: .file)
        }

        private static func color(for state: Flow.State) -> NSColor {
            switch state {
            case .active, .receiving: .systemYellow
            case .completed: .systemGreen
            case .failed: .systemRed
            case .tunnel: .systemGray
            }
        }
    }
}

/// The coloured dot in the first column.
final class StateDot: NSView {
    var color: NSColor = .systemGray {
        didSet { needsDisplay = true }
    }

    init(identifier: NSUserInterfaceItemIdentifier) {
        super.init(frame: .zero)
        self.identifier = identifier
    }

    required init?(coder: NSCoder) {
        fatalError("not used")
    }

    override func draw(_ dirtyRect: NSRect) {
        let size: CGFloat = 8
        let rect = NSRect(x: bounds.midX - size / 2, y: bounds.midY - size / 2, width: size, height: size)
        color.setFill()
        NSBezierPath(ovalIn: rect).fill()
    }
}
