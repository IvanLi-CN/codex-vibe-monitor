# Components

## 当前真相源

### 基础 primitive

`web/src/components/ui/` 是当前基础组件层的实现真相源，重点包括：

- `web/src/components/ui/button.tsx`：`default / secondary / outline / ghost / destructive` 五种按钮语义，以及 `default / sm / lg / icon` 四种尺寸。
- `web/src/components/ui/input.tsx`：标准输入框，包含边框、placeholder、focus ring、disabled 透明度约束。
- `web/src/components/ui/card.tsx`：`Card / CardHeader / CardTitle / CardDescription / CardContent / CardFooter`。
- `web/src/components/ui/badge.tsx`：`default / accent / secondary / success / info / warning / error` 七类状态徽标。
- `web/src/components/ui/alert.tsx`：`default / info / success / warning / error` 五类提示面板。
- `web/src/components/ui/switch.tsx`：布尔开关，`checked` 态使用 `primary`，`unchecked` 态停留在 `base-300`。
- `web/src/components/ui/dialog.tsx`、`web/src/components/ui/popover.tsx`、`web/src/components/ui/tooltip.tsx`、`web/src/components/ui/info-tooltip.tsx`：浮层与提示类组件。
- `web/src/components/ui/filterable-combobox.tsx`：输入+筛选+列表框的组合输入模式。
- `web/src/components/ui/form-field-feedback.tsx`、`web/src/components/ui/floating-field-error.tsx`：表单标签与错误/说明信息。
- `web/src/components/ui/spinner.tsx`：`sm / md / lg` 三档加载旋转器。

### 表单与输入反馈

- 表单标签优先使用 `.field`、`.field-label` 与 `FormFieldFeedback`，而不是每个页面自己拼标签和错误提示布局。
- 输入类组件统一使用 focus ring，而不是只依赖边框变色。
- 错误态允许在 field feedback 中出现较长文案，但输入本体仍应保持单一职责，不嵌入多段说明。

### 状态语义

- `disabled`：统一表现为不可交互 + 降低透明度，不能只保留视觉禁用而仍可点击。
- `loading`：优先使用 Spinner、占位文案或页面级 skeleton，避免按钮/输入/列表三套不一致的 loading 语义。
- `error`：优先使用 `Alert`、field-level error 或页面级错误块，不要用纯红字替代完整错误状态。
- `selected / active`：导航、分段控件、选项列表都通过边框 + 背景 + 文字强调三件套表达，不能只依靠颜色微差。

### 可访问性最低线

- 所有可点击控件必须保留 `focus-visible` 样式。
- Icon-only 交互必须提供可读 `aria-label` 或屏幕阅读器文本，例如 `DialogCloseIcon`。
- Combobox / tooltip / dialog 这类复合控件，优先沿用当前 Radix 与显式 ARIA 实现，不自己手写简化版语义。

## 后续新增规则

- 新增基础组件前，先确认能否通过现有 `web/src/components/ui/` 目录下的 primitive 组合完成；只有当模式被多处复用且现有组合已经开始重复时，才上升为基础组件。
- 新增按钮、徽标、提示变体时，必须落在既有语义色集合里；不要为了单个页面新增特例 variant。
- 新增输入控件时，必须同时定义：默认态、focus 态、disabled 态、error 态、空数据或无匹配文案。
- 所有可复用组件在进入多页面复用前，应至少具备一个 Storybook story 或可替代的独立验证入口。
- 组件文案密度保持“标题短、说明简洁、错误明确”，避免把业务解释堆进组件本身。

## 已知例外 / 待治理

- 当前 `FilterableCombobox` 仍是项目级组合组件而不是完整 command/popup 系统；如果未来出现多种复杂选择器，需要评估是否抽成更通用的选择框模式。
- 部分页面仍有直接拼 utility class 的表单区块，没有完全回收到 `web/src/components/ui/` 组件层；新增页面应优先避免继续扩大这种分叉。
- 不是所有基础组件都有独立 story；现在更多是通过页面故事反向证明它们的可用性。
