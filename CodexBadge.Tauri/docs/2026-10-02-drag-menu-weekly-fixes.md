# 2026-10-02 拖动、菜单定位与仅周卡片修复

## 用户问题与结果
- 更新窗口一拖就消失：隔离Computer Use复现，原系统非客户区拖动触发失焦隐藏，隐藏后该次拖动循环还能阻塞正常退出。第一次改鼠标轮询后不消失但快拖不移动，实际坐标验收发现IPC延迟使起点读取太晚；最终改前端pointer capture记录原始screen位移，串行合并到Rust原位置/缩放上，拖动ID隔离迟到请求。拖动中失焦不关闭，结束恢复正常关闭；显式关闭清除拖动状态。
- 移动失败也必须结束拖动：只读审查指出失败路径可能保留dragging=true，补失败回归后修复，移动异常仍发end并清空前端拖动对象。
- 托盘菜单靠近点击位置：鼠标旁2 DIP，优先上方右侧、屏幕边界翻转并clamp。仅改变托盘/无避让位置；胶囊菜单仍避让胶囊，子菜单不重新移动根菜单。
- 仅周卡片：本周剩余/百分比/进度条在上部，日期和重置机会/查看在同一底行，增加上部及底行间距，重置机会右对齐。异常仍显示状态，不制造百分比。模式取自实际额度窗口，不能按Pro标签推断；此次PRO是隔离样本。
- 日期统一`M/D HH:mm 重置`，去掉“日”；同日五小时仍用`将于 HH:mm 重置`。额度重置相关代码与web已搜索，除否定断言无残留“日”。
- 用户要求全球用户发现：建议保留手动一键更新，发布时准备英文README首屏、真实演示、准确About与topics，不堆热门词，不保证Star增长。已记录`docs/release-discoverability-checklist.md`，本次没有修改GitHub元数据或发布。

## 验收
- Rust133通过、5项手工测试默认忽略；Node38通过。拖动失焦、快速release先于begin回复、移动失败收尾、100/125/150%位移与边界、菜单指针定位、日期格式都有针对性回归。
- 仅周27组合模拟：3主题×100/125/150%×fresh/stale/unavailable；每次等内容测量完成再按实际高度调整视口，日期与重置机会同Y、无重叠/溢出。`artifacts/weekly-bottom-row-scale-acceptance.json`。
- 125%原生实际拖动前后：更新窗口`(1463,463)`→`(1763,567)`，Visible仍true；记录`artifacts/update-drag-v3-before.json`、`update-drag-v3-after.json`。另一个最终隔离窗口实际Esc后Visible=false：`artifacts/update-esc-final-observe.json`。
- 菜单指定点击点`(970,450)`，根菜单在`(973,312)`，高度135物理像素，底部离点3物理像素；边界翻转为几何用例，未操作用户真实托盘。
- 原生仅周模拟底行截图：`artifacts/submenu-2026-10-01/weekly-bottom-row-native-125.png`；菜单截图同目录`menu-pointer-native-125.png`。无真实Pro验收。
- 新签名候选在`artifacts/candidate-v0.3.2-layout-fixes-2026-10-02`，版本沿用未发布0.3.2。不自动安装/替换运行中的PID25684旧候选；WPF及无关文件保留，未提交或发布。
- 未完成原生100/150%、混合DPI拖动及实际安装升级/重启验收。STATIC_VCRUNTIME环境有弃用警告，构建仍成功。菜单正常托盘输入应由用户实际复核。
