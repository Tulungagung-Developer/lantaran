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

        for column in Column.allCases {
            let tableColumn = NSTableColumn(identifier: column.identifier)
            tableColumn.title = column.title
            tableColumn.width = column.width
            tableColumn.minWidth = 20
            if column != .state {
                // Numbers and times read best largest first, text A to Z.
                tableColumn.sortDescriptorPrototype = NSSortDescriptor(
                    key: column.rawValue, ascending: !column.sortsDescendingFirst)
            }
            table.addTableColumn(tableColumn)
        }

        // Set after the columns exist, so the saved widths, order and sort
        // apply to them.
        table.autosaveName = "TrafficTable"
        table.autosaveTableColumns = true
        if table.sortDescriptors.isEmpty {
            table.sortDescriptors = [TableSort.newestFirst]
        }

        table.dataSource = context.coordinator
        table.delegate = context.coordinator
        context.coordinator.table = table

        let menu = NSMenu()
        menu.delegate = context.coordinator
        table.menu = menu

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
        case state, id, url, client, method, status, code, time, duration, request, response, edited

        var identifier: NSUserInterfaceItemIdentifier { .init(rawValue) }

        var sortsDescendingFirst: Bool {
            [.id, .time, .code, .duration, .request, .response].contains(self)
        }

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
            case .edited: "Edited"
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
            case .edited: 60
            }
        }
    }

    @MainActor
    final class Coordinator: NSObject, NSTableViewDataSource, NSTableViewDelegate, NSMenuDelegate {
        let model: AppModel
        weak var table: NSTableView?
        /// Indexes into `model.flows`, in the order the table shows them:
        /// `model.visible` sorted by the column header the user picked.
        private var rows: [Int] = []
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
            guard table != nil else { return }
            if revision != shownRevision {
                shownRevision = revision
                reload()
            }
            selectRow(for: selected)
        }

        private func reload() {
            guard let table else { return }
            rows = TableSort.rows(model.visible, in: model.flows, by: table.sortDescriptors.first)
            table.reloadData()
            if model.followNewest, let newest = newestRow {
                table.scrollRowToVisible(newest)
            }
        }

        /// Where the most recent visible flow is in the current order.
        private var newestRow: Int? {
            guard let last = model.visible.last else { return nil }
            if rows.first == last { return 0 }
            if rows.last == last { return rows.count - 1 }
            return rows.firstIndex(of: last)
        }

        func tableView(_ tableView: NSTableView, sortDescriptorsDidChange oldDescriptors: [NSSortDescriptor]) {
            let selected = model.selectedFlowID
            reload()
            selectRow(for: selected)
            if let row = tableView.selectedRowIndexes.first {
                tableView.scrollRowToVisible(row)
            }
        }

        private func selectRow(for id: UInt64?) {
            guard let table else { return }
            let row = id.flatMap { id in rows.firstIndex { model.flows[$0].id == id } }
            let wanted = row.map { IndexSet(integer: $0) } ?? IndexSet()
            if table.selectedRowIndexes != wanted {
                settingSelection = true
                table.selectRowIndexes(wanted, byExtendingSelection: false)
                settingSelection = false
            }
        }

        // MARK: Context menu

        func menuNeedsUpdate(_ menu: NSMenu) {
            menu.removeAllItems()
            guard let table, table.clickedRow >= 0, table.clickedRow < rows.count else { return }
            let flow = model.flows[rows[table.clickedRow]]

            func add(_ title: String, _ action: Selector, enabled: Bool = true) {
                let item = NSMenuItem(title: title, action: action, keyEquivalent: "")
                item.target = self
                item.representedObject = flow.id
                item.isEnabled = enabled
                menu.addItem(item)
            }

            add("Copy URL", #selector(copyURL(_:)))
            add("Copy as cURL", #selector(copyCurl(_:)), enabled: !flow.isTunnel)
            menu.addItem(.separator())
            if flow.isTunnel {
                add(model.isDecrypted(flow.host) ? "SSL Proxying Enabled for \(flow.host)" : "Enable SSL Proxying for \(flow.host)",
                    #selector(enableSSL(_:)), enabled: !model.isDecrypted(flow.host))
            } else {
                add("Map Local…", #selector(mapLocal(_:)), enabled: flow.status != nil)
                add("Map Remote…", #selector(mapRemote(_:)))
            }
        }

        private func flow(for sender: NSMenuItem) -> Flow? {
            model.flow(withID: sender.representedObject as? UInt64)
        }

        @objc private func copyURL(_ sender: NSMenuItem) {
            guard let flow = flow(for: sender) else { return }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(flow.url, forType: .string)
        }

        @objc private func copyCurl(_ sender: NSMenuItem) {
            guard let flow = flow(for: sender) else { return }
            NSPasteboard.general.clearContents()
            NSPasteboard.general.setString(Curl.command(for: flow, body: model.body(of: flow, response: false)?.data), forType: .string)
        }

        @objc private func enableSSL(_ sender: NSMenuItem) {
            guard let flow = flow(for: sender) else { return }
            model.enableSSL(for: flow.host)
        }

        @objc private func mapLocal(_ sender: NSMenuItem) {
            guard let flow = flow(for: sender) else { return }
            model.startMapLocal(from: flow)
        }

        @objc private func mapRemote(_ sender: NSMenuItem) {
            guard let flow = flow(for: sender) else { return }
            model.startMapRemote(from: flow)
        }

        func numberOfRows(in tableView: NSTableView) -> Int {
            rows.count
        }

        func tableViewSelectionDidChange(_ notification: Notification) {
            guard !settingSelection, let table else { return }
            let row = table.selectedRow
            model.selectedFlowID = row >= 0 && row < rows.count
                ? model.flows[rows[row]].id
                : nil
            // A row picked by hand stops the list from jumping away from it.
            if row >= 0, row != newestRow {
                model.followNewest = false
            }
        }

        func tableView(_ tableView: NSTableView, viewFor tableColumn: NSTableColumn?, row: Int) -> NSView? {
            guard let tableColumn, let column = Column(rawValue: tableColumn.identifier.rawValue),
                  row < rows.count
            else { return nil }
            let flow = model.flows[rows[row]]

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
            case .client: return flow.app
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
            case .edited: return flow.mappingKind ?? ""
            }
        }

        static func duration(_ ms: Double) -> String {
            ms < 1000 ? String(format: "%.0f ms", ms) : String(format: "%.2f s", ms / 1000)
        }

        private static let sizeFormat: ByteCountFormatter = {
            let formatter = ByteCountFormatter()
            formatter.countStyle = .file
            formatter.allowsNonnumericFormatting = false
            return formatter
        }()

        static func size(_ bytes: UInt64) -> String {
            sizeFormat.string(fromByteCount: Int64(bytes))
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

/// The table's order. Rows arrive in ID order, so sorting by ID or time is
/// the arrival order or its reverse; other columns are sorted, ties kept in
/// the chosen direction of arrival.
enum TableSort {
    static let newestFirst = NSSortDescriptor(key: TrafficTable.Column.id.rawValue, ascending: false)

    static func rows(_ visible: [Int], in flows: [Flow], by descriptor: NSSortDescriptor?) -> [Int] {
        let descriptor = descriptor ?? newestFirst
        let ascending = descriptor.ascending
        guard let key = descriptor.key, let column = TrafficTable.Column(rawValue: key),
              column != .id, column != .time, column != .state
        else { return ascending ? visible : visible.reversed() }

        return visible.sorted { a, b in
            switch compare(flows[a], flows[b], by: column) {
            case .orderedSame: ascending ? a < b : a > b
            case .orderedAscending: ascending
            case .orderedDescending: !ascending
            }
        }
    }

    private static func compare(_ a: Flow, _ b: Flow, by column: TrafficTable.Column) -> ComparisonResult {
        switch column {
        case .url: a.url.localizedStandardCompare(b.url)
        case .client: a.app.localizedStandardCompare(b.app)
        case .method: a.method.compare(b.method)
        case .status: compare(rank(a.state), rank(b.state))
        case .code: compare(a.status, b.status)
        case .duration: compare(a.durationMs, b.durationMs)
        case .request: compare(a.requestSize, b.requestSize)
        case .response: compare(a.responseSize, b.responseSize)
        case .edited: (a.mappingKind ?? "").compare(b.mappingKind ?? "")
        case .state, .id, .time: .orderedSame
        }
    }

    private static func rank(_ state: Flow.State) -> Int {
        switch state {
        case .active: 0
        case .receiving: 1
        case .completed: 2
        case .failed: 3
        case .tunnel: 4
        }
    }

    /// Missing values (no response yet) sort before any value.
    private static func compare<T: Comparable>(_ a: T?, _ b: T?) -> ComparisonResult {
        switch (a, b) {
        case let (a?, b?): a < b ? .orderedAscending : a > b ? .orderedDescending : .orderedSame
        case (nil, nil): .orderedSame
        case (nil, _): .orderedAscending
        case (_, nil): .orderedDescending
        }
    }
}
