# 总结
---

- 为`TaskControlBlock`添加了调度开始时刻的标签和syscall计数器
- 为`fn syscall(...)`植入了一个`TaskManager`的hook，用于在每次syscall调用时更新处于执行态的app的syscall调用次数