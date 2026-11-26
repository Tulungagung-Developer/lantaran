import SwiftUI

/// Which part of a flow the text filter looks at.
enum FilterField: String, CaseIterable, Identifiable {
    case any = "URL, Method, Status"
    case url = "URL"
    case method = "Method"
    case status = "Status"
    case header = "Header"
    var id: String { rawValue }
}

/// One-click filters by protocol and content type. Selecting several shows
/// flows that match any of them; selecting none shows everything.
enum QuickFilter: String, CaseIterable, Identifiable {
    case http = "HTTP"
    case https = "HTTPS"
    case websocket = "WebSocket"
    case json = "JSON"
    case form = "Form"
    case xml = "XML"
    case js = "JS"
    case css = "CSS"
    case graphql = "GraphQL"
    case document = "Document"
    case media = "Media"
    case other = "Other"
    var id: String { rawValue }

    func matches(_ flow: Flow) -> Bool {
        switch self {
        case .http: return !flow.isHTTPS
        case .https: return flow.isHTTPS
        case .websocket: return flow.isWebSocket
        case .graphql: return flow.isGraphQL
        case .other: return Self.contentKind(of: flow) == nil && !flow.isWebSocket && !flow.isGraphQL
        default: return Self.contentKind(of: flow) == self
        }
    }

    /// The content filter a flow falls under, from its response type, or its
    /// request type for forms.
    static func contentKind(of flow: Flow) -> QuickFilter? {
        let request = flow.requestContentType?.lowercased() ?? ""
        if request.contains("x-www-form-urlencoded") || request.contains("multipart/form-data") {
            return .form
        }
        let type = flow.responseContentType?.lowercased() ?? ""
        if type.contains("json") { return .json }
        if type.contains("html") { return .document }
        if type.contains("xml") { return .xml }
        if type.contains("javascript") || type.contains("ecmascript") { return .js }
        if type.contains("text/css") { return .css }
        if ["image/", "video/", "audio/", "font/"].contains(where: type.hasPrefix) { return .media }
        return nil
    }
}

/// The bar above the traffic table: a text filter with a choice of field,
/// and the quick filters.
struct FilterBar: View {
    @Environment(AppModel.self) private var model
    @FocusState private var focused: Bool

    var body: some View {
        @Bindable var model = model

        HStack(spacing: 8) {
            HStack(spacing: 4) {
                Image(systemName: "line.3.horizontal.decrease.circle")
                    .foregroundStyle(.secondary)
                Picker("", selection: $model.filterField) {
                    ForEach(FilterField.allCases) { Text($0.rawValue).tag($0) }
                }
                .labelsHidden()
                .pickerStyle(.menu)
                .fixedSize()
                .controlSize(.small)
                TextField("Filter requests (⇧⌘F)", text: $model.filterText)
                    .textFieldStyle(.plain)
                    .focused($focused)
                    .frame(minWidth: 140)
                if !model.filterText.isEmpty {
                    Button { model.filterText = "" } label: {
                        Image(systemName: "xmark.circle.fill").foregroundStyle(.secondary)
                    }
                    .buttonStyle(.plain)
                    .help("Clear the filter")
                }
            }
            .padding(.horizontal, 6)
            .padding(.vertical, 3)
            .background(.quaternary.opacity(0.5), in: RoundedRectangle(cornerRadius: 6))
            .frame(maxWidth: 360)

            ScrollView(.horizontal, showsIndicators: false) {
                HStack(spacing: 2) {
                    chip("All", selected: model.quickFilters.isEmpty) { model.quickFilters = [] }
                    Divider().frame(height: 14)
                    ForEach(QuickFilter.allCases) { filter in
                        chip(filter.rawValue, selected: model.quickFilters.contains(filter)) {
                            model.toggle(filter)
                        }
                    }
                }
            }
        }
        .padding(.horizontal, 8)
        .padding(.vertical, 5)
        .background(.bar)
        .onChange(of: model.focusFilterRequest) { focused = true }
    }

    private func chip(_ title: String, selected: Bool, action: @escaping () -> Void) -> some View {
        Button(action: action) {
            Text(title)
                .font(.callout)
                .padding(.horizontal, 8)
                .padding(.vertical, 2)
                .foregroundStyle(selected ? Color.white : Color.primary)
                .background(selected ? Color.accentColor : Color.clear, in: Capsule())
                .contentShape(Capsule())
        }
        .buttonStyle(.plain)
    }
}
